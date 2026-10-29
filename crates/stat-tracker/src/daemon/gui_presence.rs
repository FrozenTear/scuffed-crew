//! Is a tracker GUI running? The daemon writes `live_snapshot.json` only
//! for a running GUI (plus after a finished game, at startup and at
//! shutdown), so it needs to know without asking the GUI.
//!
//! The GUI holds an exclusive `flock` on `scuffed-stat-tracker-gui.lock` in
//! `$XDG_RUNTIME_DIR` (or the data dir when that is unset or empty) for its
//! whole process lifetime, also while it sits in the tray
//! (`stat-tracker-ui/src/instance.rs`). The kernel lists every flock in
//! `/proc/locks` by device and inode, so the daemon looks the file up there:
//! read only, it never takes the lock and cannot race the GUI's own claim.
//!
//! Any doubt counts as present (a probe error costs one small write; a
//! wrong "absent" would leave the GUI stale). A missing lock file is absent.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Same name as `stat_tracker_ui::instance::LOCK_FILE`; a test reads the
/// GUI source to keep them equal (the lib cannot depend on the UI).
pub(crate) const GUI_LOCK_FILE: &str = "scuffed-stat-tracker-gui.lock";

/// How long one probe answer is reused.
const CACHE_FOR: Duration = Duration::from_secs(1);

/// The lock file paths a GUI for `data_dir` may use, as the GUI picks them
/// (`lock_dir` in the GUI): the runtime dir when set and non-empty, else
/// the data dir. Both are probed: the daemon's environment can differ from
/// the GUI's.
pub(crate) fn lock_paths(runtime_dir: Option<std::ffi::OsString>, data_dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(dir) = runtime_dir.filter(|d| !d.is_empty()) {
        paths.push(PathBuf::from(dir).join(GUI_LOCK_FILE));
    }
    let in_data = data_dir.join(GUI_LOCK_FILE);
    if !paths.contains(&in_data) {
        paths.push(in_data);
    }
    paths
}

/// Linux `dev_t` major and minor numbers (glibc encoding).
fn dev_major_minor(dev: u64) -> (u64, u64) {
    let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff);
    let minor = (dev & 0xff) | ((dev >> 12) & !0xff);
    (major, minor)
}

/// `(major, minor, inode)` of every FLOCK line in `/proc/locks` text.
/// A line reads `1: FLOCK  ADVISORY  WRITE 1234 fd:01:5678 0 EOF`; a waiter
/// line has `->` after the number. Numbers are hex for the device, decimal
/// for the inode.
pub(crate) fn flocked_files(locks: &str) -> Vec<(u64, u64, u64)> {
    locks
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace().skip(1).filter(|p| *p != "->");
            if parts.next()? != "FLOCK" {
                return None;
            }
            let id = parts.find(|p| p.matches(':').count() == 2)?;
            let mut it = id.split(':');
            let major = u64::from_str_radix(it.next()?, 16).ok()?;
            let minor = u64::from_str_radix(it.next()?, 16).ok()?;
            let inode = it.next()?.parse().ok()?;
            Some((major, minor, inode))
        })
        .collect()
}

/// One probe. `locks` is the `/proc/locks` text, `None` when unreadable.
pub(crate) fn probe(paths: &[PathBuf], locks: Option<&str>) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    let mut files = Vec::new();
    for path in paths {
        match std::fs::metadata(path) {
            Ok(meta) => {
                let (major, minor) = dev_major_minor(meta.dev());
                files.push((major, minor, meta.ino()));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return true,
        }
    }
    if files.is_empty() {
        return false;
    }
    let Some(locks) = locks else {
        return true;
    };
    let held = flocked_files(locks);
    files.iter().any(|f| held.contains(f))
}

static CACHE: std::sync::Mutex<Option<(Instant, PathBuf, bool)>> = std::sync::Mutex::new(None);

/// Whether a GUI for `data_dir` is running, cached for [`CACHE_FOR`].
pub(crate) fn gui_present(data_dir: &Path) -> bool {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, dir, present)) = cache.as_ref()
        && at.elapsed() < CACHE_FOR
        && dir == data_dir
    {
        return *present;
    }
    let paths = lock_paths(std::env::var_os("XDG_RUNTIME_DIR"), data_dir);
    let locks = std::fs::read_to_string("/proc/locks").ok();
    let present = probe(&paths, locks.as_deref());
    *cache = Some((Instant::now(), data_dir.to_path_buf(), present));
    present
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_name_matches_the_gui() {
        let gui = include_str!("../../../stat-tracker-ui/src/instance.rs");
        assert!(
            gui.contains(&format!("pub const LOCK_FILE: &str = \"{GUI_LOCK_FILE}\";")),
            "the GUI lock file was renamed; update GUI_LOCK_FILE"
        );
        assert!(
            gui.contains("XDG_RUNTIME_DIR"),
            "the GUI no longer picks the runtime dir; update lock_paths"
        );
    }

    #[test]
    fn both_lock_dirs_are_probed() {
        let data = Path::new("/d");
        assert_eq!(
            lock_paths(Some("/run/user/1000".into()), data),
            vec![
                PathBuf::from("/run/user/1000").join(GUI_LOCK_FILE),
                data.join(GUI_LOCK_FILE)
            ]
        );
        assert_eq!(
            lock_paths(Some("".into()), data),
            vec![data.join(GUI_LOCK_FILE)]
        );
        assert_eq!(lock_paths(None, data), vec![data.join(GUI_LOCK_FILE)]);
    }

    #[test]
    fn proc_locks_lines_parse() {
        let text = "1: FLOCK  ADVISORY  WRITE 4242 fd:01:131075 0 EOF\n\
                    2: POSIX  ADVISORY  WRITE 99 00:1a:77 0 EOF\n\
                    3: -> FLOCK  ADVISORY  WRITE 4243 00:2b:9 0 EOF\n\
                    4: OFDLCK ADVISORY  READ  -1 00:05:12 0 EOF\n\
                    garbage\n";
        assert_eq!(
            flocked_files(text),
            vec![(0xfd, 0x01, 131075), (0x00, 0x2b, 9)]
        );
    }

    #[test]
    fn no_lock_file_is_absent_and_an_unreadable_table_is_present() {
        let dir = tempfile::tempdir().unwrap();
        let paths = lock_paths(None, dir.path());
        assert!(!probe(&paths, Some("")));
        assert!(!probe(&paths, None), "no file at all: no GUI");
        std::fs::write(&paths[0], b"").unwrap();
        assert!(!probe(&paths, Some("")), "a stale file nobody locks");
        assert!(probe(&paths, None), "file there, table unreadable: present");
    }

    /// A real flock in this process shows up in the real `/proc/locks`.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_held_flock_is_seen_and_a_released_one_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let paths = lock_paths(None, dir.path());
        let file = std::fs::File::create(&paths[0]).unwrap();
        file.try_lock().expect("lock the test file");
        let locks = std::fs::read_to_string("/proc/locks").unwrap();
        assert!(probe(&paths, Some(&locks)), "held lock seen");
        file.unlock().unwrap();
        let locks = std::fs::read_to_string("/proc/locks").unwrap();
        assert!(!probe(&paths, Some(&locks)), "released lock gone");
    }
}
