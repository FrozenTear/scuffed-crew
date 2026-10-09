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

const PARTIAL_GRACE: Duration = Duration::from_secs(3600);

pub fn reports_dir_from_env() -> PathBuf {
    match std::env::var("REPORTS_DIR") {
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value.trim()),
        _ => PathBuf::from("data/reports"),
    }
}

/// True when the reports directory could be served or could overlap uploads.
pub fn reports_dir_conflicts(reports: &Path, upload_dir: &Path) -> bool {
    if same_or_nested(reports, upload_dir) {
        return true;
    }
    let dist = Path::new("dist");
    reports == dist || reports.starts_with(dist)
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

pub async fn remove_report_files(dir: &Path, id: &str) {
    if !is_report_id(id) {
        return;
    }
    let _ = tokio::fs::remove_file(report_zip_path(dir, id)).await;
    let _ = tokio::fs::remove_file(report_partial_path(dir, id)).await;
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
        remove_report_files(reports_dir, &report.id).await;
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
                if !id_set.contains(id)
                    && partial_is_stale(&entry).await
                    && tokio::fs::remove_file(entry.path()).await.is_ok()
                {
                    outcome.orphan_files += 1;
                }
                continue;
            }
            if let Some(id) = name.strip_suffix(".zip")
                && is_report_id(id)
                && !id_set.contains(id)
                && tokio::fs::remove_file(entry.path()).await.is_ok()
            {
                outcome.orphan_files += 1;
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

async fn partial_is_stale(entry: &tokio::fs::DirEntry) -> bool {
    let Ok(meta) = entry.metadata().await else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    modified.elapsed().unwrap_or_default() > PARTIAL_GRACE
}
