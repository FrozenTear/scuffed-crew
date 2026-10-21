//! Hourly purge of personal stats whose soft-delete deadline has passed.
//!
//! The first tick runs at startup. Each due member is wiped in its own
//! transaction, which skips the member if a restore cleared or moved the
//! deadline. Report files for that member are removed as soon as the
//! transaction commits. A later member's error does not skip those files
//! and does not stop the rest of the list. The account stays.

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
            let now = chrono::Utc::now();
            match db.members_due_for_stats_purge(now).await {
                Ok(member_ids) => {
                    apply_due_purges(&db, &cache, &reports_dir, &member_ids, now).await;
                }
                Err(error) => {
                    tracing::error!(error_kind = purge_error_kind(&error), "stats purge failed");
                }
            }
        }
    });
}

/// Wipe each listed member. Files of a committed wipe are removed before the
/// next member is attempted.
pub(crate) async fn apply_due_purges(
    db: &Database,
    cache: &LeaderboardCache,
    reports_dir: &std::path::Path,
    member_ids: &[String],
    now: chrono::DateTime<chrono::Utc>,
) {
    let mut members = 0u64;
    let mut personal_match = 0u64;
    let mut stat_report = 0u64;
    for member_id in member_ids {
        match db.purge_member_if_due(member_id, now).await {
            Ok(Some(wipe)) => {
                members += 1;
                personal_match += wipe.personal_match;
                stat_report += wipe.stat_report;
                for id in wipe.stat_report_ids {
                    if let Err(error) = remove_report_files(reports_dir, &id).await {
                        tracing::warn!(
                            error_kind = ?error.kind(),
                            "stat report file remove after purge"
                        );
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    member_id = %member_id,
                    error_kind = purge_error_kind(&error),
                    "stats purge member failed"
                );
            }
        }
    }
    if members == 0 {
        return;
    }
    cache.invalidate_all();
    tracing::info!(
        members,
        personal_match,
        stat_report,
        "purged scheduled personal stats"
    );
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
        scuffed_db::DbError::ReservedName => "reserved_name",
    }
}

#[cfg(test)]
mod tests {
    use scuffed_db::{DELETED_MEMBER_ID, OrgRole};

    use super::apply_due_purges;

    const REPORT_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const REPORT_C: &str = "cccccccccccccccccccccccccccccccc";

    async fn report_row(db: &scuffed_db::Database, member_id: &str, report_id: &str) {
        db.client
            .query(format!(
                "CREATE stat_report:{report_id} SET \
                    member_id = $mid, created_at = time::now(), reason_category = 'other', \
                    reason_text = 'x', app_version = '0', recognizer_matcher = 'm', \
                    recognizer_ocr = 'o', training_consent = false, own_name_included = false, \
                    glyphs_included = false, size_bytes = 1, zip_sha256 = 'abc'"
            ))
            .bind(("mid", member_id.to_string()))
            .await
            .unwrap()
            .check()
            .unwrap();
    }

    fn personal_match(member_id: &str, session_id: &str) -> scuffed_db::PersonalMatch {
        let now = chrono::Utc::now();
        scuffed_db::PersonalMatch {
            id: String::new(),
            member_id: member_id.to_string(),
            session_id: session_id.to_string(),
            hero: "Ana".into(),
            map_name: "Busan".into(),
            game_mode: "control".into(),
            role: "Support".into(),
            outcome: "victory".into(),
            elims: 1,
            deaths: 1,
            assists: 1,
            damage: 1,
            healing: 1,
            mitigation: 0,
            played_at: now,
            uploaded_at: now,
            edited: false,
            recognizer: "ocr-v1".into(),
            name_recognizer: None,
            suspect_fields: Vec::new(),
        }
    }

    #[tokio::test]
    async fn earlier_member_files_are_removed_when_a_later_purge_fails() {
        let state = crate::test_support::test_state().await;
        let dir = std::env::temp_dir().join("scuffed-purge-partial-failure");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let early = state
            .db
            .create_member("user-early", "Early", OrgRole::Member)
            .await
            .unwrap();
        let later = state
            .db
            .create_member("user-later", "Later", OrgRole::Member)
            .await
            .unwrap();
        state
            .db
            .upsert_personal_matches(&early.id, &[personal_match(&early.id, "early-game")])
            .await
            .unwrap();
        state
            .db
            .upsert_personal_matches(&later.id, &[personal_match(&later.id, "later-game")])
            .await
            .unwrap();
        report_row(&state.db, &early.id, REPORT_A).await;
        report_row(&state.db, &later.id, REPORT_C).await;
        let early_zip = dir.join(format!("{REPORT_A}.zip"));
        let later_zip = dir.join(format!("{REPORT_C}.zip"));
        tokio::fs::write(&early_zip, b"early").await.unwrap();
        tokio::fs::write(&later_zip, b"later").await.unwrap();

        let base = chrono::Utc::now();
        let (deadline, _) = state
            .db
            .schedule_stats_delete(&early.id, base)
            .await
            .unwrap();
        state
            .db
            .schedule_stats_delete(&later.id, base)
            .await
            .unwrap();

        apply_due_purges(
            &state.db,
            &state.leaderboard_cache,
            &dir,
            &[
                early.id.clone(),
                DELETED_MEMBER_ID.to_string(),
                later.id.clone(),
            ],
            deadline,
        )
        .await;

        assert!(!early_zip.exists(), "committed wipe removes its zip");
        assert!(
            !later_zip.exists(),
            "the pass continues after a member error"
        );
        let early_member = state.db.get_member(&early.id).await.unwrap().unwrap();
        let later_member = state.db.get_member(&later.id).await.unwrap().unwrap();
        assert!(early_member.stats_delete_scheduled_at.is_none());
        assert!(later_member.stats_delete_scheduled_at.is_none());
        assert_eq!(
            state
                .db
                .get_personal_stats(&early.id)
                .await
                .unwrap()
                .total_matches,
            0
        );
        assert_eq!(
            state
                .db
                .get_personal_stats(&later.id)
                .await
                .unwrap()
                .total_matches,
            0
        );
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
