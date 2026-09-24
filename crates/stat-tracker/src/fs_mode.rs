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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

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
