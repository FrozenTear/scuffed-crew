//! One tracker GUI per user session (#234).
//!
//! The first GUI takes an exclusive `flock` on
//! `$XDG_RUNTIME_DIR/scuffed-stat-tracker-gui.lock` (the data dir when
//! `XDG_RUNTIME_DIR` is unset) and keeps the file open until it exits. A
//! second launch finds the lock held, touches `<data_dir>/show_window` and
//! exits 0. The running GUI polls that file's mtime on its Tick and shows
//! its window, the same path as the tray "Show" item.
//!
//! The kernel drops a `flock` when the holder dies, including on SIGKILL or
//! a crash, so a stale lock file never blocks a launch. Any error while
//! taking the lock lets the launch go ahead: a second window is better than
//! no window.

use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

pub const LOCK_FILE: &str = "scuffed-stat-tracker-gui.lock";
pub const SHOW_FILE: &str = "show_window";

/// Outcome of [`claim`].
#[derive(Debug)]
pub enum Claim {
    /// This process is the GUI. Keep the file open for the process lifetime.
    Primary(File),
    /// Another GUI holds the lock.
    Secondary,
}

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

/// Try to become the one GUI. Never blocks.
pub fn claim(lock_path: &Path) -> io::Result<Claim> {
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    match file.try_lock() {
        Ok(()) => Ok(Claim::Primary(file)),
        Err(TryLockError::WouldBlock) => Ok(Claim::Secondary),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

pub fn show_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SHOW_FILE)
}

/// Ask the running GUI to show its window: move `<data_dir>/show_window`'s
/// mtime to now.
pub fn request_show(data_dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(show_path(data_dir))?;
    file.set_modified(SystemTime::now())
}

/// Watches `<data_dir>/show_window`. The first poll only records the
/// current state, so a file left by an earlier session does not pop the
/// window at startup. After that, a file that appears or whose mtime moves
/// fires once.
#[derive(Debug, Default)]
pub struct ShowWatch {
    /// `None` until the first poll; then the last seen mtime (`None` = no file).
    seen: Option<Option<SystemTime>>,
}

impl ShowWatch {
    pub fn poll(&mut self, data_dir: &Path) -> bool {
        let mtime = std::fs::metadata(show_path(data_dir))
            .and_then(|m| m.modified())
            .ok();
        let fire = match self.seen {
            None => false,
            Some(prev) => mtime.is_some() && mtime != prev,
        };
        self.seen = Some(mtime);
        fire
    }
}

static SHOW_WATCH: Mutex<ShowWatch> = Mutex::new(ShowWatch { seen: None });

/// Process-wide [`ShowWatch::poll`] for the GUI Tick. True when a second
/// launch asked for the window since the last call.
pub fn consume_show(data_dir: &Path) -> bool {
    SHOW_WATCH
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .poll(data_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const CHILD_ENV: &str = "STAT_TRACKER_UI_TEST_LOCK_HOLDER";

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
        let first = claim(&path).unwrap();
        assert!(matches!(first, Claim::Primary(_)));
        assert!(matches!(claim(&path).unwrap(), Claim::Secondary));
        assert!(matches!(claim(&path).unwrap(), Claim::Secondary));
        drop(first);
        assert!(matches!(claim(&path).unwrap(), Claim::Primary(_)));
    }

    #[test]
    fn stale_lock_file_does_not_block() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        std::fs::write(&path, b"12345\n").unwrap();
        assert!(matches!(claim(&path).unwrap(), Claim::Primary(_)));
    }

    /// Helper process for `killed_holder_does_not_block_launch`. Does
    /// nothing unless the parent test sets the env var.
    #[test]
    #[ignore]
    fn lock_holder_child() {
        let Some(path) = std::env::var_os(CHILD_ENV) else {
            return;
        };
        let path = PathBuf::from(path);
        let claim = claim(&path).unwrap();
        assert!(matches!(claim, Claim::Primary(_)));
        std::fs::write(path.with_extension("ready"), b"").unwrap();
        std::thread::sleep(Duration::from_secs(60));
        drop(claim);
    }

    #[test]
    fn killed_holder_does_not_block_launch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        let ready = path.with_extension("ready");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "instance::tests::lock_holder_child",
                "--exact",
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD_ENV, &path)
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
        assert!(
            matches!(claim(&path).unwrap(), Claim::Secondary),
            "a live holder must keep the lock"
        );
        child.kill().unwrap(); // SIGKILL: no destructors, no unlock call.
        child.wait().unwrap();
        assert!(
            matches!(claim(&path).unwrap(), Claim::Primary(_)),
            "a killed holder must not block the next launch"
        );
    }

    #[test]
    fn show_watch_ignores_startup_then_fires_on_request() {
        let dir = tempfile::tempdir().unwrap();
        let mut watch = ShowWatch::default();
        assert!(!watch.poll(dir.path()), "first poll only records state");
        assert!(!watch.poll(dir.path()));
        request_show(dir.path()).unwrap();
        assert!(watch.poll(dir.path()), "a new file fires");
        assert!(!watch.poll(dir.path()), "fires once per request");
        std::thread::sleep(Duration::from_millis(20));
        request_show(dir.path()).unwrap();
        assert!(watch.poll(dir.path()), "a moved mtime fires");
        assert!(!watch.poll(dir.path()));
    }

    #[test]
    fn show_watch_ignores_file_left_by_earlier_session() {
        let dir = tempfile::tempdir().unwrap();
        request_show(dir.path()).unwrap();
        let mut watch = ShowWatch::default();
        assert!(!watch.poll(dir.path()));
        assert!(!watch.poll(dir.path()));
    }
}
