//! Private storage and the hourly retention sweep for tracker bug reports.
//!
//! Files live under `REPORTS_DIR` (default `data/reports`). That directory is
//! not the upload tree and it is not served as static files.

mod bundle;
mod png;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub use bundle::{AcceptedBundle, BundleReject, accept_bundle, sha256_hex};

use scuffed_db::Database;

/// Orphan zips and stale partials younger than this are left for the next sweep.
const FILE_GRACE: Duration = Duration::from_secs(10 * 60);

pub fn reports_dir_from_env() -> PathBuf {
    match std::env::var("REPORTS_DIR") {
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value.trim()),
        _ => PathBuf::from("data/reports"),
    }
}

/// True when the reports directory could be served or could overlap uploads.
///
/// Both paths are canonicalized first, so a symlink into uploads or `dist` counts.
pub fn reports_dir_conflicts(reports: &Path, upload_dir: &Path) -> bool {
    let reports = canonicalize_for_check(reports);
    let upload_dir = canonicalize_for_check(upload_dir);
    if same_or_nested(&reports, &upload_dir) {
        return true;
    }
    let dist = canonicalize_for_check(Path::new("dist"));
    reports == dist || reports.starts_with(&dist)
}

/// Open `REPORTS_DIR`. On failure the caller keeps the site up and skips reports.
pub async fn open_reports_dir(configured: &Path, upload_dir: &Path) -> (PathBuf, bool) {
    match try_open_reports_dir(configured, upload_dir).await {
        Ok(dir) => (dir, true),
        Err(reason) => {
            tracing::error!("stat reports disabled: {reason}");
            (configured.to_path_buf(), false)
        }
    }
}

async fn try_open_reports_dir(configured: &Path, upload_dir: &Path) -> Result<PathBuf, String> {
    ensure_reports_dir(configured).await.map_err(|error| {
        format!(
            "REPORTS_DIR {} is missing or not writable ({error})",
            configured.display()
        )
    })?;
    let probe = configured.join(".write-probe");
    if let Err(error) = tokio::fs::write(&probe, b"ok").await {
        return Err(format!(
            "REPORTS_DIR {} is not writable ({error})",
            configured.display()
        ));
    }
    let _ = tokio::fs::remove_file(&probe).await;
    let reports = std::fs::canonicalize(configured).map_err(|error| {
        format!(
            "REPORTS_DIR {} could not be resolved ({error})",
            configured.display()
        )
    })?;
    if reports_dir_conflicts(&reports, upload_dir) {
        return Err(format!(
            "REPORTS_DIR {} overlaps the upload directory or the web root",
            reports.display()
        ));
    }
    Ok(reports)
}

fn canonicalize_for_check(path: &Path) -> PathBuf {
    if let Ok(canon) = path.canonicalize() {
        return canon;
    }
    let mut suffix = Vec::new();
    let mut cursor = path.to_path_buf();
    loop {
        if let Ok(canon) = cursor.canonicalize() {
            let mut out = canon;
            for part in suffix.iter().rev() {
                out.push(part);
            }
            return out;
        }
        let Some(parent) = cursor.parent() else {
            return path.to_path_buf();
        };
        if parent == cursor {
            return path.to_path_buf();
        }
        if let Some(name) = cursor.file_name() {
            suffix.push(name.to_os_string());
        }
        cursor = parent.to_path_buf();
    }
}

fn same_or_nested(a: &Path, b: &Path) -> bool {
    a == b || a.starts_with(b) || b.starts_with(a)
}

pub async fn ensure_reports_dir(dir: &Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).await?;
    }
    Ok(())
}

pub fn is_report_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn report_zip_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.zip"))
}

pub fn report_partial_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.zip.partial"))
}

/// Remove the zip and any partial. `NotFound` is success. Any other error is returned
/// so the caller can keep the database row and retry on the next sweep.
pub async fn remove_report_files(dir: &Path, id: &str) -> std::io::Result<()> {
    if !is_report_id(id) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "report id is not valid",
        ));
    }
    remove_if_present(&report_zip_path(dir, id)).await?;
    remove_if_present(&report_partial_path(dir, id)).await?;
    Ok(())
}

async fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn spawn_sweeper(db: Arc<Database>, reports_dir: PathBuf) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        loop {
            interval.tick().await;
            if let Err(err) = sweep_stat_reports(&db, &reports_dir).await {
                tracing::error!("stat report sweep failed: {err}");
            }
        }
    });
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepOutcome {
    pub expired: usize,
    pub orphan_files: usize,
    pub missing_rows: usize,
}

/// Delete expired reports, zip files with no row, and rows with no file.
pub async fn sweep_stat_reports(db: &Database, reports_dir: &Path) -> Result<SweepOutcome, String> {
    let mut outcome = SweepOutcome::default();
    let now = chrono::Utc::now();
    let expired = db
        .list_expired_stat_reports(now)
        .await
        .map_err(|err| err.to_string())?;
    for report in expired {
        if let Err(error) = remove_report_files(reports_dir, &report.id).await {
            tracing::error!(%error, report_id = %report.id, "stat report file delete failed");
            continue;
        }
        if db
            .delete_stat_report(&report.id)
            .await
            .map_err(|err| err.to_string())?
        {
            outcome.expired += 1;
        }
    }

    let ids = db
        .list_stat_report_ids()
        .await
        .map_err(|err| err.to_string())?;
    let id_set: std::collections::BTreeSet<&str> = ids.iter().map(String::as_str).collect();

    if tokio::fs::metadata(reports_dir).await.is_ok() {
        let mut dir = tokio::fs::read_dir(reports_dir)
            .await
            .map_err(|err| err.to_string())?;
        while let Some(entry) = dir.next_entry().await.map_err(|err| err.to_string())? {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if let Some(id) = name.strip_suffix(".zip.partial") {
                if !is_report_id(id) {
                    continue;
                }
                if id_set.contains(id) && !report_zip_path(reports_dir, id).exists() {
                    let _ = tokio::fs::rename(
                        report_partial_path(reports_dir, id),
                        report_zip_path(reports_dir, id),
                    )
                    .await;
                    continue;
                }
                if !id_set.contains(id) && file_is_stale(&entry).await {
                    match tokio::fs::remove_file(entry.path()).await {
                        Ok(()) => outcome.orphan_files += 1,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            tracing::error!(
                                %error,
                                report_id = %id,
                                "stat report file delete failed"
                            );
                        }
                    }
                }
                continue;
            }
            if let Some(id) = name.strip_suffix(".zip")
                && is_report_id(id)
                && !id_set.contains(id)
                && file_is_stale(&entry).await
            {
                match tokio::fs::remove_file(entry.path()).await {
                    Ok(()) => outcome.orphan_files += 1,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        tracing::error!(%error, report_id = %id, "stat report file delete failed");
                    }
                }
            }
        }
    }

    for id in &ids {
        if !is_report_id(id) {
            continue;
        }
        let zip_exists = report_zip_path(reports_dir, id).exists();
        let partial_exists = report_partial_path(reports_dir, id).exists();
        if !zip_exists
            && !partial_exists
            && db
                .delete_stat_report(id)
                .await
                .map_err(|err| err.to_string())?
        {
            outcome.missing_rows += 1;
        }
    }
    Ok(outcome)
}

async fn file_is_stale(entry: &tokio::fs::DirEntry) -> bool {
    let Ok(meta) = entry.metadata().await else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    modified.elapsed().unwrap_or_default() > FILE_GRACE
}
