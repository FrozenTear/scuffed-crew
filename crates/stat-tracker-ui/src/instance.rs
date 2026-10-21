//! One tracker GUI per user session (#234).
//!
//! The first GUI takes an exclusive `flock` on
//! `$XDG_RUNTIME_DIR/scuffed-stat-tracker-gui.lock` (the data dir when
//! `XDG_RUNTIME_DIR` is unset), writes its pid and version into it, and keeps
//! the file open until it exits.
//!
//! A later launch that finds the lock held:
//! - shows the running GUI: it touches `<data_dir>/show_window` and exits 0;
//! - or, when it replaces the running GUI (`--replace` from "Restart now", or
//!   the holder is an older binary), touches `<data_dir>/quit_window` and
//!   waits a few seconds for the lock.
//!
//! The running GUI polls both files on its Tick. `show_window` runs the tray
//! "Show" path and `quit_window` runs the tray "Quit" path. While an in-app
//! update runs, the GUI leaves `quit_window` unpolled: quitting would drop
//! the update, which stops the installer half way with the tracker stopped.
//!
//! A replacing launch removes `quit_window` once its wait ends, whether it
//! got the lock or not, and a new primary removes both request files left
//! by an earlier session. So a stale request never fires, even after the
//! wall clock steps back past its mtime.
//!
//! The kernel drops a `flock` when the holder dies, including on SIGKILL or
//! a crash, so a stale lock file never blocks a launch. Any error while
//! taking the lock lets the launch go ahead: a second window is better than
//! no window.

use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

pub const LOCK_FILE: &str = "scuffed-stat-tracker-gui.lock";
/// The GUI binary under `<prefix>/bin`, as `install.sh` installs it.
pub const GUI_BIN: &str = "stat-tracker-gui";
pub const SHOW_FILE: &str = "show_window";
pub const QUIT_FILE: &str = "quit_window";

/// How long a replacing launch waits for the old GUI to exit.
pub const REPLACE_WAIT: Duration = Duration::from_secs(10);

/// Outcome of [`claim`].
#[derive(Debug)]
pub enum Claim {
    /// This process is the GUI. Keep the file open for the process lifetime.
    Primary(File),
    /// Another GUI holds the lock.
    Secondary,
}

/// The GUI that holds the lock, as written into the lock file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    pub version: String,
}

/// When this process became the primary GUI. Requests older than this were
/// meant for the previous holder.
static ACQUIRED_AT: OnceLock<SystemTime> = OnceLock::new();

/// Directory for the lock file: a non-empty `XDG_RUNTIME_DIR`, else the
/// data dir.
pub fn lock_dir(runtime_dir: Option<OsString>, data_dir: &Path) -> PathBuf {
    match runtime_dir {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => data_dir.to_path_buf(),
    }
}

pub fn lock_path(runtime_dir: Option<OsString>, data_dir: &Path) -> PathBuf {
    lock_dir(runtime_dir, data_dir).join(LOCK_FILE)
}

/// Try to become the one GUI. Never blocks. On success the lock file holds
/// `<pid>\n<version>\n`.
pub fn claim(lock_path: &Path, version: &str) -> io::Result<Claim> {
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    match file.try_lock() {
        Ok(()) => {
            let _ = ACQUIRED_AT.set(SystemTime::now());
            // Best effort: a holder the next launch cannot read is treated
            // as current, which only means "show it".
            let _ = file
                .set_len(0)
                .and_then(|()| file.rewind())
                .and_then(|()| write!(file, "{}\n{}\n", std::process::id(), version.trim()))
                .and_then(|()| file.flush());
            Ok(Claim::Primary(file))
        }
        Err(TryLockError::WouldBlock) => Ok(Claim::Secondary),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

/// The launch decision for `main`: take the lock, or replace an older
/// holder (`replace` is `--replace` from "Restart now"), or report
/// [`Claim::Secondary`] so the caller asks the holder to show. A new primary
/// removes request files left by an earlier session.
pub fn claim_session(
    lock_path: &Path,
    data_dir: &Path,
    version: &str,
    replace: bool,
    proc_root: &Path,
    wait: Duration,
) -> io::Result<Claim> {
    let claimed = match claim(lock_path, version)? {
        Claim::Secondary => {
            // Replace the running GUI on "Restart now" or when it is an
            // older binary (left over from an update). Otherwise the caller
            // just brings its window up.
            let older = read_holder(lock_path)
                .is_some_and(|holder| holder_is_older(&holder, version, proc_root));
            if replace || older {
                tracing::info!(lock = %lock_path.display(), older, "replacing the running tracker GUI");
                claim_replacing(lock_path, data_dir, version, wait)?
            } else {
                Claim::Secondary
            }
        }
        primary => primary,
    };
    if matches!(claimed, Claim::Primary(_)) {
        clear_stale_requests(data_dir);
    }
    Ok(claimed)
}

/// Ask the holder to quit, then retry [`claim`] until `wait` runs out.
/// The quit request is removed when the wait ends, with or without the
/// lock: this launch is the only one that needed it. Left behind, it would
/// make a holder that answers late quit with no GUI to follow it, or make a
/// later GUI quit after the wall clock stepped back past its mtime.
pub fn claim_replacing(
    lock_path: &Path,
    data_dir: &Path,
    version: &str,
    wait: Duration,
) -> io::Result<Claim> {
    request_quit(data_dir)?;
    let start = Instant::now();
    let claimed = loop {
        match claim(lock_path, version) {
            Ok(Claim::Secondary) if start.elapsed() < wait => {
                std::thread::sleep(Duration::from_millis(100));
            }
            other => break other,
        }
    };
    let _ = std::fs::remove_file(quit_path(data_dir));
    claimed
}

/// Remove `show_window` and `quit_window`. A new primary calls this right
/// after taking the lock, so any request file it sees later was written for
/// it, whatever the wall clock did since. Best effort: a file that cannot be
/// removed is still filtered by its mtime ([`RequestWatch`]).
pub fn clear_stale_requests(data_dir: &Path) {
    for path in [show_path(data_dir), quit_path(data_dir)] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                tracing::warn!(error = %e, path = %path.display(), "could not remove a stale GUI request");
            }
        }
    }
}

pub fn read_holder(lock_path: &Path) -> Option<Holder> {
    let mut raw = String::new();
    File::open(lock_path).ok()?.read_to_string(&mut raw).ok()?;
    let mut lines = raw.lines();
    let pid = lines.next()?.trim().parse().ok()?;
    let version = lines.next().unwrap_or_default().trim().to_string();
    Some(Holder { pid, version })
}

/// True when the holder runs an older binary than `my_version`: its
/// executable was replaced on disk (`/proc/<pid>/exe` ends in
/// ` (deleted)`), or its recorded version is lower. An unreadable or equal
/// holder is not older, so the launch only shows it.
pub fn holder_is_older(holder: &Holder, my_version: &str, proc_root: &Path) -> bool {
    let exe = proc_root.join(holder.pid.to_string()).join("exe");
    if let Ok(target) = std::fs::read_link(&exe)
        && target.to_string_lossy().ends_with(" (deleted)")
    {
        return true;
    }
    match (
        crate::update::parse_semver(&holder.version),
        crate::update::parse_semver(my_version),
    ) {
        (Some(theirs), Some(mine)) => theirs < mine,
        _ => false,
    }
}

/// Path to start the new binary from. After an update `/proc/self/exe`
/// reads `<path> (deleted)`; the new file sits at `<path>`.
pub fn relaunch_path(current_exe: &Path) -> PathBuf {
    let raw = current_exe.to_string_lossy();
    match raw.strip_suffix(" (deleted)") {
        Some(path) => PathBuf::from(path),
        None => current_exe.to_path_buf(),
    }
}

/// Binary for "Restart now": `<install_prefix>/bin/stat-tracker-gui`, the
/// file the update just installed, when it exists. Otherwise (no prefix, no
/// file there) the running binary's path, as [`relaunch_path`] gives it.
/// The two differ when `STAT_TRACKER_PREFIX` points elsewhere or this GUI
/// runs from outside a `bin/` directory.
pub fn relaunch_target(install_prefix: Option<&Path>, current_exe: &Path) -> PathBuf {
    let running = relaunch_path(current_exe);
    if let Some(prefix) = install_prefix {
        let installed = prefix.join("bin").join(GUI_BIN);
        if installed != running && installed.is_file() {
            return installed;
        }
    }
    running
}

/// Start the installed GUI binary with this process's arguments plus
/// `--replace`, so it waits for this process to exit and takes the lock.
/// `install_prefix` is the prefix the in-app update installed to.
pub fn spawn_replacement(install_prefix: Option<&Path>) -> io::Result<()> {
    let exe = relaunch_target(install_prefix, &std::env::current_exe()?);
    let mut args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if !args.iter().any(|a| a == "--replace") {
        args.push("--replace".into());
    }
    std::process::Command::new(exe).args(args).spawn()?;
    Ok(())
}

pub fn show_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SHOW_FILE)
}

pub fn quit_path(data_dir: &Path) -> PathBuf {
    data_dir.join(QUIT_FILE)
}

fn touch(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.set_modified(SystemTime::now())
}

/// Ask the running GUI to show its window.
pub fn request_show(data_dir: &Path) -> io::Result<()> {
    touch(&show_path(data_dir))
}

/// Ask the running GUI to quit (tray Quit path).
pub fn request_quit(data_dir: &Path) -> io::Result<()> {
    touch(&quit_path(data_dir))
}

/// Watches one request file. The first poll fires when the file's mtime is
/// newer than `since` (a request that came in after this GUI took the lock
/// but before its first Tick). After that, a file that appears or whose
/// mtime moves fires once. A file left by an earlier session never fires.
#[derive(Debug)]
pub struct RequestWatch {
    path: PathBuf,
    since: SystemTime,
    /// `None` until the first poll; then the last seen mtime (`None` = no file).
    seen: Option<Option<SystemTime>>,
}

impl RequestWatch {
    pub fn new(path: PathBuf, since: SystemTime) -> Self {
        Self {
            path,
            since,
            seen: None,
        }
    }

    pub fn poll(&mut self) -> bool {
        let mtime = std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok();
        let fire = match self.seen {
            None => mtime.is_some_and(|t| t > self.since),
            Some(prev) => mtime.is_some() && mtime != prev,
        };
        self.seen = Some(mtime);
        fire
    }
}

static SHOW_WATCH: Mutex<Option<RequestWatch>> = Mutex::new(None);
static QUIT_WATCH: Mutex<Option<RequestWatch>> = Mutex::new(None);

fn consume(slot: &Mutex<Option<RequestWatch>>, path: PathBuf) -> bool {
    // Only the GUI that holds the lock answers requests. A fixture run or a
    // launch that could not open the lock leaves them alone.
    let Some(since) = ACQUIRED_AT.get().copied() else {
        return false;
    };
    let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(|| RequestWatch::new(path, since))
        .poll()
}

/// For the GUI Tick: true when a later launch asked to show the window.
pub fn consume_show(data_dir: &Path) -> bool {
    consume(&SHOW_WATCH, show_path(data_dir))
}

/// For the GUI Tick: true when a newer GUI asked this one to quit.
pub fn consume_quit(data_dir: &Path) -> bool {
    consume(&QUIT_WATCH, quit_path(data_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};

    const CHILD_ENV: &str = "STAT_TRACKER_UI_TEST_LOCK_HOLDER";
    const CHILD_MODE_ENV: &str = "STAT_TRACKER_UI_TEST_LOCK_MODE";

    fn spawn_holder(path: &Path, mode: &str) -> Child {
        let ready = path.with_extension("ready");
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "instance::tests::lock_holder_child",
                "--exact",
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD_ENV, path)
            .env(CHILD_MODE_ENV, mode)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let start = Instant::now();
        while !ready.exists() {
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "lock holder never started"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        child
    }

    /// Helper process for the tests below. Does nothing unless the parent
    /// test sets the env var. Mode `sleep` holds the lock for 60 s. Mode
    /// `quit` behaves like the GUI: it exits when a quit request arrives.
    #[test]
    #[ignore]
    fn lock_holder_child() {
        let Some(path) = std::env::var_os(CHILD_ENV) else {
            return;
        };
        let path = PathBuf::from(path);
        let data_dir = path.parent().unwrap().to_path_buf();
        let claim = claim(&path, "0.5.0-alpha.5.1").unwrap();
        assert!(matches!(claim, Claim::Primary(_)));
        std::fs::write(path.with_extension("ready"), b"").unwrap();
        let quit_mode = std::env::var(CHILD_MODE_ENV).as_deref() == Ok("quit");
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(60) {
            if quit_mode && consume_quit(&data_dir) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(claim);
    }

    #[test]
    fn lock_dir_prefers_runtime_dir() {
        let data = Path::new("/data");
        assert_eq!(
            lock_path(Some("/run/user/7".into()), data),
            Path::new("/run/user/7").join(LOCK_FILE)
        );
        assert_eq!(lock_path(Some("".into()), data), data.join(LOCK_FILE));
        assert_eq!(lock_path(None, data), data.join(LOCK_FILE));
    }

    #[test]
    fn second_claim_is_secondary_until_first_drops() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join(LOCK_FILE);
        let first = claim(&path, "1.0.0").unwrap();
        assert!(matches!(first, Claim::Primary(_)));
        assert!(matches!(claim(&path, "1.0.0").unwrap(), Claim::Secondary));
        assert!(matches!(claim(&path, "1.0.0").unwrap(), Claim::Secondary));
        drop(first);
        // Other tests spawn child processes in parallel. Between fork and
        // exec a child shares this process's open files, so the flock can
        // outlive `drop` by a moment. Retry briefly.
        let start = Instant::now();
        let mut again = claim(&path, "1.0.0").unwrap();
        while matches!(again, Claim::Secondary) && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(10));
            again = claim(&path, "1.0.0").unwrap();
        }
        assert!(matches!(again, Claim::Primary(_)));
    }

    #[test]
    fn primary_records_pid_and_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        std::fs::write(&path, b"999999\n9.9.9-long-stale-version\n").unwrap();
        let _held = claim(&path, " 0.5.0-alpha.5.1 ").unwrap();
        assert_eq!(
            read_holder(&path),
            Some(Holder {
                pid: std::process::id(),
                version: "0.5.0-alpha.5.1".into()
            })
        );
    }

    #[test]
    fn stale_lock_file_does_not_block() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        std::fs::write(&path, b"12345\n0.1.0\n").unwrap();
        assert!(matches!(claim(&path, "1.0.0").unwrap(), Claim::Primary(_)));
    }

    #[test]
    fn killed_holder_does_not_block_launch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        let mut child = spawn_holder(&path, "sleep");
        assert!(
            matches!(claim(&path, "1.0.0").unwrap(), Claim::Secondary),
            "a live holder must keep the lock"
        );
        child.kill().unwrap(); // SIGKILL: no destructors, no unlock call.
        child.wait().unwrap();
        assert!(
            matches!(claim(&path, "1.0.0").unwrap(), Claim::Primary(_)),
            "a killed holder must not block the next launch"
        );
    }

    #[test]
    fn replacing_launch_quits_the_holder_and_takes_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        let mut child = spawn_holder(&path, "quit");
        let holder = read_holder(&path).expect("holder wrote its pid");
        assert_eq!(holder.pid, child.id());
        assert!(matches!(claim(&path, "1.0.0").unwrap(), Claim::Secondary));
        let start = Instant::now();
        let got = claim_replacing(&path, dir.path(), "1.0.0", Duration::from_secs(20)).unwrap();
        assert!(
            matches!(got, Claim::Primary(_)),
            "the holder must quit on request and free the lock"
        );
        assert!(start.elapsed() < Duration::from_secs(15));
        let status = child.wait().unwrap();
        assert!(status.success(), "holder exited cleanly: {status:?}");
        assert_eq!(read_holder(&path).unwrap().pid, std::process::id());
        assert!(
            !quit_path(dir.path()).exists(),
            "R2-2: the request is removed once the replace succeeded"
        );
    }

    #[test]
    fn replacing_launch_gives_up_on_a_stuck_holder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        let mut child = spawn_holder(&path, "sleep");
        let got = claim_replacing(&path, dir.path(), "1.0.0", Duration::from_millis(300)).unwrap();
        assert!(matches!(got, Claim::Secondary));
        assert!(
            !quit_path(dir.path()).exists(),
            "R2-2: a launch that gave up takes its request back, so the holder does not quit later with no GUI to follow"
        );
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn a_new_primary_ignores_requests_from_before_a_clock_step() {
        // R2-2: request files left by an earlier session, with mtimes an
        // hour "ahead" because the wall clock stepped back since.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        let ahead = SystemTime::now() + Duration::from_secs(3600);
        for stale in [quit_path(dir.path()), show_path(dir.path())] {
            std::fs::write(&stale, b"").unwrap();
            File::options()
                .write(true)
                .open(&stale)
                .unwrap()
                .set_modified(ahead)
                .unwrap();
        }
        let now = SystemTime::now();
        assert!(
            RequestWatch::new(quit_path(dir.path()), now).poll(),
            "precondition: by mtime alone the stale request would fire"
        );
        let got = claim_session(
            &path,
            dir.path(),
            "1.0.0",
            false,
            Path::new("/nonexistent-proc"),
            Duration::from_millis(300),
        )
        .unwrap();
        assert!(matches!(got, Claim::Primary(_)));
        let mut quit = RequestWatch::new(quit_path(dir.path()), now);
        let mut show = RequestWatch::new(show_path(dir.path()), now);
        assert!(!quit.poll(), "a stale quit request must not fire");
        assert!(!show.poll(), "a stale show request must not fire");
        std::thread::sleep(Duration::from_millis(20));
        request_quit(dir.path()).unwrap();
        assert!(quit.poll(), "a request written after the claim still fires");
    }

    #[test]
    fn a_same_version_launch_only_shows_the_holder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        let mut child = spawn_holder(&path, "quit");
        let got = claim_session(
            &path,
            dir.path(),
            "0.5.0-alpha.5.1",
            false,
            Path::new("/nonexistent-proc"),
            Duration::from_millis(300),
        )
        .unwrap();
        assert!(matches!(got, Claim::Secondary));
        assert!(!quit_path(dir.path()).exists(), "no quit request was sent");
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn restart_starts_the_binary_the_update_installed() {
        // R2-3: the update installs to `<prefix>/bin/stat-tracker-gui`.
        let prefix = tempfile::tempdir().unwrap();
        let installed = prefix.path().join("bin").join(GUI_BIN);
        let elsewhere = Path::new("/opt/dev/target/release/stat-tracker-gui (deleted)");
        assert_eq!(
            relaunch_target(Some(prefix.path()), elsewhere),
            Path::new("/opt/dev/target/release/stat-tracker-gui"),
            "no file at the prefix: fall back to the running binary"
        );
        std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
        std::fs::write(&installed, b"#!/bin/sh\n").unwrap();
        assert_eq!(relaunch_target(Some(prefix.path()), elsewhere), installed);
        let running = PathBuf::from(format!("{} (deleted)", installed.display()));
        assert_eq!(
            relaunch_target(Some(prefix.path()), &running),
            installed,
            "same file: the running path, without the suffix"
        );
        assert_eq!(
            relaunch_target(None, elsewhere),
            Path::new("/opt/dev/target/release/stat-tracker-gui")
        );
    }

    #[test]
    fn holder_is_older_by_deleted_exe_or_version() {
        let proc_root = tempfile::tempdir().unwrap();
        let pid_dir = proc_root.path().join("42");
        std::fs::create_dir_all(&pid_dir).unwrap();
        let same = Holder {
            pid: 42,
            version: "0.5.0-alpha.5.1".into(),
        };

        std::os::unix::fs::symlink("/home/u/.local/bin/stat-tracker-gui", pid_dir.join("exe"))
            .unwrap();
        assert!(!holder_is_older(&same, "0.5.0-alpha.5.1", proc_root.path()));
        assert!(
            !holder_is_older(&same, "0.5.0-alpha.5", proc_root.path()),
            "a newer holder is shown, not replaced"
        );
        assert!(holder_is_older(&same, "0.5.0-alpha.6", proc_root.path()));

        std::fs::remove_file(pid_dir.join("exe")).unwrap();
        std::os::unix::fs::symlink(
            "/home/u/.local/bin/stat-tracker-gui (deleted)",
            pid_dir.join("exe"),
        )
        .unwrap();
        assert!(
            holder_is_older(&same, "0.5.0-alpha.5.1", proc_root.path()),
            "a replaced binary is older even at the same version"
        );

        let unknown = Holder {
            pid: 7,
            version: String::new(),
        };
        assert!(!holder_is_older(&unknown, "0.5.0", proc_root.path()));
    }

    #[test]
    fn relaunch_path_strips_deleted_suffix() {
        assert_eq!(
            relaunch_path(Path::new("/home/u/.local/bin/stat-tracker-gui (deleted)")),
            Path::new("/home/u/.local/bin/stat-tracker-gui")
        );
        assert_eq!(
            relaunch_path(Path::new("/usr/bin/stat-tracker-gui")),
            Path::new("/usr/bin/stat-tracker-gui")
        );
    }

    #[test]
    fn request_watch_ignores_startup_then_fires_on_request() {
        let dir = tempfile::tempdir().unwrap();
        let since = SystemTime::now();
        let mut watch = RequestWatch::new(show_path(dir.path()), since);
        assert!(!watch.poll(), "no file yet");
        assert!(!watch.poll());
        std::thread::sleep(Duration::from_millis(20));
        request_show(dir.path()).unwrap();
        assert!(watch.poll(), "a new file fires");
        assert!(!watch.poll(), "fires once per request");
        std::thread::sleep(Duration::from_millis(20));
        request_show(dir.path()).unwrap();
        assert!(watch.poll(), "a moved mtime fires");
        assert!(!watch.poll());
    }

    #[test]
    fn request_watch_ignores_file_left_by_earlier_session() {
        let dir = tempfile::tempdir().unwrap();
        request_show(dir.path()).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let mut watch = RequestWatch::new(show_path(dir.path()), SystemTime::now());
        assert!(!watch.poll());
        assert!(!watch.poll());
    }

    #[test]
    fn request_before_first_poll_fires() {
        // R241-B: a launch between taking the lock and the first Tick.
        let dir = tempfile::tempdir().unwrap();
        let since = SystemTime::now();
        std::thread::sleep(Duration::from_millis(20));
        request_show(dir.path()).unwrap();
        let mut watch = RequestWatch::new(show_path(dir.path()), since);
        assert!(watch.poll(), "a request newer than the lock fires");
        assert!(!watch.poll());
    }

    #[test]
    fn quit_request_is_consumed_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut watch = RequestWatch::new(quit_path(dir.path()), SystemTime::now());
        assert!(!watch.poll());
        std::thread::sleep(Duration::from_millis(20));
        request_quit(dir.path()).unwrap();
        assert!(watch.poll());
        assert!(!watch.poll());
        assert!(!watch.poll());
    }
}
