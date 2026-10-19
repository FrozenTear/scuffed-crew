//! Hourly purge of personal stats whose soft-delete deadline has passed.
//!
//! The first tick runs at startup. The purge deletes stats rows only. The
//! account stays, and `stats_delete_scheduled_at` is cleared so the member
//! can upload again. Report zip files are removed best-effort after the
//! database write.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use scuffed_db::Database;

use crate::leaderboard_cache::LeaderboardCache;
use crate::stat_reports::remove_report_files;

/// How often due soft deletes are hard-deleted.
pub const STATS_PURGE_INTERVAL: Duration = Duration::from_secs(3600);

pub fn spawn_stats_purge(db: Arc<Database>, cache: LeaderboardCache, reports_dir: PathBuf) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(STATS_PURGE_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            match db.purge_scheduled_stats(chrono::Utc::now()).await {
                Ok(outcome) => {
                    if outcome.members == 0 {
                        continue;
                    }
                    cache.invalidate_all();
                    tracing::info!(
                        members = outcome.members,
                        personal_match = outcome.personal_match,
                        stat_report = outcome.stat_report,
                        "purged scheduled personal stats"
                    );
                    for id in outcome.stat_report_ids {
                        if let Err(error) = remove_report_files(&reports_dir, &id).await {
                            tracing::warn!(
                                error_kind = ?error.kind(),
                                "stat report file remove after purge"
                            );
                        }
                    }
                }
                Err(error) => {
                    tracing::error!(error_kind = purge_error_kind(&error), "stats purge failed");
                }
            }
        }
    });
}

fn purge_error_kind(error: &scuffed_db::DbError) -> &'static str {
    match error {
        scuffed_db::DbError::Surreal(_) => "surreal",
        scuffed_db::DbError::NotFound(_) => "not_found",
        scuffed_db::DbError::Conflict(_) => "conflict",
        scuffed_db::DbError::Config(_) => "config",
        scuffed_db::DbError::Timeout => "timeout",
        scuffed_db::DbError::Crypto(_) => "crypto",
        scuffed_db::DbError::DisplayNameInvalid => "display_name_invalid",
        scuffed_db::DbError::DisplayNameTaken => "display_name_taken",
    }
}
