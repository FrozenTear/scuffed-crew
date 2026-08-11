//! Startup / maintenance helpers around the local SurrealKV store and logs.
//!
//! SurrealKV keeps historical versions; weeks of captures leave the store
//! dominated by dead data (see [`super::LocalStore::vacuum`]). Manual
//! `--vacuum` already compact, but nothing ran unless the user remembered.
//! These helpers run at daemon start (pid-guard held, store not yet open):
//! rotate `daemon.log`, prune old pre-vacuum backups, and auto-vacuum when
//! the live store exceeds a size threshold.

use std::path::{Path, PathBuf};

use super::LocalStore;

/// Live store directory name under `data_dir`.
pub const STORE_DIR_NAME: &str = "stats.surrealkv";

/// Prefix for vacuum backup directories: `stats.surrealkv.pre-vacuum-<stamp>`.
pub const PRE_VACUUM_BACKUP_PREFIX: &str = "stats.surrealkv.pre-vacuum-";

/// Auto-vacuum at startup when the live store is larger than this.
///
/// After a healthy vacuum, ~8k matches sat near ~8 MB; the bloat regime that
/// drove multi-hundred-MB RSS / swap was tens of MB and up. 32 MiB is well
/// above a compact store and well below the painful zone.
pub const AUTO_VACUUM_THRESHOLD_BYTES: u64 = 32 * 1024 * 1024;

/// Rotate `daemon.log` when it exceeds this size (GUI-spawned daemons append
/// here; systemd units use the journal instead).
pub const DAEMON_LOG_MAX_BYTES: u64 = 32 * 1024 * 1024;

/// Keep this many newest `pre-vacuum-*` backups; delete the rest.
pub const PRE_VACUUM_BACKUPS_KEEP: usize = 1;

/// Best-effort recursive size of a file or directory tree.
pub fn dir_size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::metadata(path) else {
        return 0;
    };
    if meta.is_file() {
        return meta.len();
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries.flatten().map(|e| dir_size(&e.path())).sum()
}

/// Path of the live store under `data_dir`.
pub fn store_live_path(data_dir: &Path) -> PathBuf {
    data_dir.join(STORE_DIR_NAME)
}

/// Byte size of the live store (0 if missing).
pub fn store_size_bytes(data_dir: &Path) -> u64 {
    let p = store_live_path(data_dir);
    if p.exists() { dir_size(&p) } else { 0 }
}

/// List pre-vacuum backup paths, newest first (by stamp in the directory name).
pub fn list_pre_vacuum_backups(data_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(data_dir) else {
        return Vec::new();
    };
    let mut backups: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(PRE_VACUUM_BACKUP_PREFIX))
        })
        .collect();
    // Stamp is sortable ISO-like `YYYYMMDD_HHMMSS` — reverse name = newest first.
    backups.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    backups
}

/// Delete older pre-vacuum backups, keeping the `keep` newest. Returns how
/// many entries were removed.
pub fn prune_pre_vacuum_backups(data_dir: &Path, keep: usize) -> std::io::Result<usize> {
    let backups = list_pre_vacuum_backups(data_dir);
    if backups.len() <= keep {
        return Ok(0);
    }
    let mut removed = 0usize;
    for path in backups.into_iter().skip(keep) {
        let res = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match res {
            Ok(()) => removed += 1,
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "failed to remove old pre-vacuum backup"
                );
            }
        }
    }
    Ok(removed)
}

/// If `daemon.log` exceeds `max_bytes`, rename it to `daemon.log.1` (replacing
/// any previous `.1`). Returns `true` when a rotation happened.
pub fn rotate_daemon_log(data_dir: &Path, max_bytes: u64) -> std::io::Result<bool> {
    let log_path = data_dir.join("daemon.log");
    let meta = match std::fs::metadata(&log_path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    if meta.len() <= max_bytes {
        return Ok(false);
    }
    let rotated = data_dir.join("daemon.log.1");
    let _ = std::fs::remove_file(&rotated);
    std::fs::rename(&log_path, &rotated)?;
    Ok(true)
}

/// Run non-blocking-ish maintenance under the pid guard, before the daemon
/// opens the store: log rotation, backup prune, optional auto-vacuum.
pub async fn startup_maintenance(
    data_dir: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match rotate_daemon_log(data_dir, DAEMON_LOG_MAX_BYTES) {
        Ok(true) => tracing::info!(
            max_mb = DAEMON_LOG_MAX_BYTES / (1024 * 1024),
            "rotated daemon.log (exceeded max size; previous kept as daemon.log.1)"
        ),
        Ok(false) => {}
        Err(e) => tracing::warn!(error = %e, "daemon.log rotation failed"),
    }

    match prune_pre_vacuum_backups(data_dir, PRE_VACUUM_BACKUPS_KEEP) {
        Ok(n) if n > 0 => tracing::info!(
            pruned = n,
            keep = PRE_VACUUM_BACKUPS_KEEP,
            "pruned old pre-vacuum backups"
        ),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "pre-vacuum backup prune failed"),
    }

    let size = store_size_bytes(data_dir);
    if size == 0 {
        return Ok(());
    }
    if size <= AUTO_VACUUM_THRESHOLD_BYTES {
        tracing::debug!(
            size_mb = size as f64 / 1e6,
            threshold_mb = AUTO_VACUUM_THRESHOLD_BYTES as f64 / 1e6,
            "store under auto-vacuum threshold"
        );
        return Ok(());
    }

    tracing::info!(
        size_mb = format!("{:.1}", size as f64 / 1e6),
        threshold_mb = AUTO_VACUUM_THRESHOLD_BYTES / (1024 * 1024),
        "auto-vacuum: live store over threshold — compacting"
    );
    let before = size;
    let (matches, sessions, tombstones) = LocalStore::vacuum(data_dir).await?;
    let after = store_size_bytes(data_dir);
    // Vacuum just created a new backup; enforce keep-1 again.
    if let Ok(n) = prune_pre_vacuum_backups(data_dir, PRE_VACUUM_BACKUPS_KEEP)
        && n > 0
    {
        tracing::info!(pruned = n, "pruned pre-vacuum backups after auto-vacuum");
    }
    tracing::info!(
        matches,
        sessions,
        tombstones,
        before_mb = format!("{:.1}", before as f64 / 1e6),
        after_mb = format!("{:.1}", after as f64 / 1e6),
        "auto-vacuum complete"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn prune_keeps_newest_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        for stamp in ["20260101_000000", "20260811_120000", "20260301_000000"] {
            let p = root.join(format!("{PRE_VACUUM_BACKUP_PREFIX}{stamp}"));
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("marker"), stamp).unwrap();
        }
        // Unrelated path must survive.
        std::fs::create_dir_all(root.join("debug")).unwrap();

        let removed = prune_pre_vacuum_backups(root, 1).unwrap();
        assert_eq!(removed, 2);

        let left = list_pre_vacuum_backups(root);
        assert_eq!(left.len(), 1);
        assert!(
            left[0]
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with("20260811_120000"),
            "newest stamp must be kept: {:?}",
            left[0]
        );
        assert!(root.join("debug").is_dir());
    }

    #[test]
    fn prune_noop_when_at_or_under_keep() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::create_dir_all(root.join(format!("{PRE_VACUUM_BACKUP_PREFIX}only"))).unwrap();
        assert_eq!(prune_pre_vacuum_backups(root, 1).unwrap(), 0);
        assert_eq!(list_pre_vacuum_backups(root).len(), 1);
    }

    #[test]
    fn rotate_log_when_over_max() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let log = root.join("daemon.log");
        {
            let mut f = std::fs::File::create(&log).unwrap();
            f.write_all(&[b'x'; 100]).unwrap();
        }
        assert!(rotate_daemon_log(root, 50).unwrap());
        assert!(!log.exists());
        assert_eq!(
            std::fs::metadata(root.join("daemon.log.1")).unwrap().len(),
            100
        );
        // Second rotation replaces .1
        {
            let mut f = std::fs::File::create(&log).unwrap();
            f.write_all(&[b'y'; 80]).unwrap();
        }
        assert!(rotate_daemon_log(root, 50).unwrap());
        assert_eq!(
            std::fs::metadata(root.join("daemon.log.1")).unwrap().len(),
            80
        );
    }

    #[test]
    fn rotate_skips_small_or_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        assert!(!rotate_daemon_log(root, 100).unwrap());
        std::fs::write(root.join("daemon.log"), b"hi").unwrap();
        assert!(!rotate_daemon_log(root, 100).unwrap());
        assert!(root.join("daemon.log").exists());
    }

    #[test]
    fn dir_size_sums_tree() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/b/f"), [0u8; 10]).unwrap();
        std::fs::write(root.join("a/g"), [0u8; 5]).unwrap();
        assert_eq!(dir_size(&root.join("a")), 15);
    }
}
