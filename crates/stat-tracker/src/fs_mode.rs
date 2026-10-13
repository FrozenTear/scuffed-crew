//! Owner-only modes for the tracker data dir, the local store, and the
//! GUI→daemon command queue.
//!
//! The store is a SurrealKV directory (`stats.surrealkv`), not a SQLite
//! file. New files the daemon creates also inherit umask 077 (set in
//! `main`). These helpers fix directories and files that already exist
//! with a looser mode.

use std::path::Path;

/// Directory mode 0700 (owner rwx). No-op off Unix.
pub fn tighten_private_dir(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// File mode 0600 (owner rw). No-op off Unix.
pub fn tighten_private_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// `tighten_private_dir` on `root` and every real subdirectory, and
/// `tighten_private_file` on every regular file. Symlinks are left alone
/// so a link cannot point chmod at a path outside the tree.
pub fn tighten_private_tree(root: &Path) {
    tighten_private_dir(root);
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        if ft.is_dir() {
            tighten_private_tree(&path);
        } else if ft.is_file() {
            tighten_private_file(&path);
        }
    }
}

/// Set this process's `RLIMIT_CORE` to 0, soft and hard, so a crash never
/// writes a core file: the daemon holds full unblanked frames and the report
/// kit's memory, and a core would put them on disk. The systemd unit sets
/// `LimitCORE=0` too; this covers a daemon the GUI starts outside systemd.
/// The hard limit 0 cannot be raised again by this process or its children.
/// With `core_pattern` piped to systemd-coredump, the helper reads this limit
/// and stores nothing.
///
/// `PR_SET_DUMPABLE` 0 is not used: it also makes `/proc/<pid>/exe`
/// unreadable to the GUI, which identifies the daemon by it
/// ([`crate::proc_id`]).
///
/// Async-signal-safe (one `setrlimit` call), so it may run in `pre_exec`.
pub fn disable_core_dumps() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let zero = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `setrlimit` reads the one `rlimit` passed by pointer and
        // only changes this process's resource limit.
        if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &zero) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn disable_core_dumps_sets_both_core_limits_to_zero() {
        // R2-3 on #236. Run it in a child (after fork, before exec) so the
        // test process keeps its limits; the limit is inherited across exec.
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.args(["-c", "ulimit -S -c; ulimit -H -c"]);
        // SAFETY: the closure makes only async-signal-safe calls
        // (`getrlimit`, `setrlimit`) and allocates nothing.
        unsafe {
            cmd.pre_exec(|| {
                // Raise the soft limit to the hard one first, so a host
                // default of soft 0 does not hide a no-op.
                let mut cur = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                if libc::getrlimit(libc::RLIMIT_CORE, &mut cur) == 0 {
                    cur.rlim_cur = cur.rlim_max;
                    libc::setrlimit(libc::RLIMIT_CORE, &cur);
                }
                disable_core_dumps()
            });
        }
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "{out:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "0\n0\n");
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn tree_tightens_dirs_to_0700_and_files_to_0600_and_skips_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let nested = root.join("stats.surrealkv");
        let commands = root.join("commands");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(&commands).unwrap();
        let db_file = nested.join("LOG");
        let cmd_file = commands.join("cmd.json");
        std::fs::write(&db_file, b"x").unwrap();
        std::fs::write(&cmd_file, b"{}").unwrap();
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&commands, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&db_file, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(&cmd_file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let link = root.join("escape");
        std::os::unix::fs::symlink("/tmp", &link).unwrap();

        tighten_private_tree(root);

        assert_eq!(mode(root), 0o700);
        assert_eq!(mode(&nested), 0o700);
        assert_eq!(mode(&commands), 0o700);
        assert_eq!(mode(&db_file), 0o600);
        assert_eq!(mode(&cmd_file), 0o600);
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    }
}
