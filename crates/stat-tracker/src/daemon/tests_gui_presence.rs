//! Tests for `gui_presence.rs`.

use std::path::{Path, PathBuf};

use super::gui_presence::*;

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

/// stat() and /proc/locks can name different devices for one file (btrfs
/// subvolumes report their own device to stat). The same inode under any
/// device counts as held: in doubt, present.
#[test]
fn an_inode_match_on_another_device_counts_as_present() {
    let dir = tempfile::tempdir().unwrap();
    let paths = lock_paths(None, dir.path());
    std::fs::write(&paths[0], b"").unwrap();
    let ino = {
        use std::os::unix::fs::MetadataExt as _;
        std::fs::metadata(&paths[0]).unwrap().ino()
    };
    let line = format!("1: FLOCK  ADVISORY  WRITE 4242 ff:ff:{ino} 0 EOF\n");
    assert!(probe(&paths, Some(&line)));
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
