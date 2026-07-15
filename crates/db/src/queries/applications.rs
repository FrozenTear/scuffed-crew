use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::{Application, ApplicationStatus};
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbApplication {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    user_id: String,
    status: String,
    preferred_games: Vec<String>,
    preferred_roles: Vec<String>,
    message: Option<String>,
    reviewed_by: Option<String>,
    review_notes: Option<String>,
    trial_started_at: Option<SurrealDatetime>,
    trial_ends_at: Option<SurrealDatetime>,
    mentor_id: Option<String>,
    created_at: SurrealDatetime,
    updated_at: SurrealDatetime,
}

fn parse_status(s: &str) -> ApplicationStatus {
    match s {
        "pending" => ApplicationStatus::Pending,
        "trial" => ApplicationStatus::Trial,
        "accepted" => ApplicationStatus::Accepted,
        "rejected" => ApplicationStatus::Rejected,
        "withdrawn" => ApplicationStatus::Withdrawn,
        _ => ApplicationStatus::Pending,
    }
}

fn db_to_application(db: DbApplication) -> Application {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    Application {
        id,
        user_id: db.user_id,
        status: parse_status(&db.status),
        preferred_games: db.preferred_games,
        preferred_roles: db.preferred_roles,
        message: db.message,
        reviewed_by: db.reviewed_by,
        review_notes: db.review_notes,
        trial_started_at: db.trial_started_at.map(|d| d.into()),
        trial_ends_at: db.trial_ends_at.map(|d| d.into()),
        mentor_id: db.mentor_id,
        created_at: db.created_at.into(),
        updated_at: db.updated_at.into(),
    }
}

impl Database {
    pub async fn submit_application(
        &self,
        user_id: &str,
        preferred_games: Vec<String>,
        preferred_roles: Vec<String>,
        message: Option<&str>,
    ) -> DbResult<Application> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let db_app = DbApplication {
                id: None,
                user_id: user_id.to_string(),
                status: "pending".to_string(),
                preferred_games,
                preferred_roles,
                message: message.map(|s| s.to_string()),
                reviewed_by: None,
                review_notes: None,
                trial_started_at: None,
                trial_ends_at: None,
                mentor_id: None,
                created_at: now,
                updated_at: now,
            };
            let created: Option<DbApplication> =
                self.client.create("application").content(db_app).await?;
            Ok(db_to_application(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create application".into())
            })?))
        })
        .await
    }

    /// Count applications still in the open pipeline (pending or trial).
    pub async fn count_open_applications(&self, user_id: &str) -> DbResult<u64> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct CountResult {
                count: u64,
            }
            let mut result = self
                .client
                .query(
                    "SELECT count() FROM application WHERE user_id = $uid AND status IN ['pending', 'trial'] GROUP ALL",
                )
                .bind(("uid", user_id.to_string()))
                .await?;
            let counts: Vec<CountResult> = result.take(0)?;
            Ok(counts.first().map(|c| c.count).unwrap_or(0))
        })
        .await
    }

    /// Delete an application by id (used to roll back concurrent double-submit).
    pub async fn delete_application(&self, id: &str) -> DbResult<()> {
        with_timeout(async {
            let _: Option<DbApplication> = self.client.delete(("application", id)).await?;
            Ok(())
        })
        .await
    }

    /// List applications newest-first with pagination (`limit + 1` for next-page detection).
    pub async fn list_applications_paginated(
        &self,
        limit: u32,
        offset: u32,
    ) -> DbResult<Vec<Application>> {
        with_timeout(async {
            let fetch = limit + 1;
            let mut result = self
                .client
                .query(
                    "SELECT * FROM application ORDER BY created_at DESC LIMIT $lim START $off",
                )
                .bind(("lim", fetch as i64))
                .bind(("off", offset as i64))
                .await?;
            let apps: Vec<DbApplication> = result.take(0)?;
            Ok(apps.into_iter().map(db_to_application).collect())
        })
        .await
    }

    /// List all applications (hard-capped). Prefer [`Self::list_applications_paginated`].
    pub async fn list_applications(&self) -> DbResult<Vec<Application>> {
        // Cap unbounded admin list to avoid full-table dumps under load.
        self.list_applications_paginated(500, 0).await
    }

    pub async fn get_application_by_user(&self, user_id: &str) -> DbResult<Option<Application>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM application WHERE user_id = $uid ORDER BY created_at DESC LIMIT 1")
                .bind(("uid", user_id.to_string()))
                .await?;
            let apps: Vec<DbApplication> = result.take(0)?;
            Ok(apps.into_iter().next().map(db_to_application))
        })
        .await
    }

    /// Get an application by record id.
    pub async fn get_application(&self, id: &str) -> DbResult<Option<Application>> {
        with_timeout(async {
            let row: Option<DbApplication> = self.client.select(("application", id)).await?;
            Ok(row.map(db_to_application))
        })
        .await
    }

    /// Update application status with **atomic** compare-and-swap on the current status.
    ///
    /// Uses a single `UPDATE … WHERE status = $expected` so concurrent officer
    /// updates cannot both succeed. Returns [`crate::DbError::Conflict`] if the
    /// expected status does not match at write time.
    pub async fn update_application_status(
        &self,
        id: &str,
        expected_status: ApplicationStatus,
        status: ApplicationStatus,
        reviewed_by: &str,
        review_notes: Option<&str>,
    ) -> DbResult<Application> {
        with_timeout(async {
            let now = Utc::now();
            let rid = RecordId::new("application", id);

            let (trial_started, trial_ends) = if status == ApplicationStatus::Trial {
                (
                    Some(SurrealDatetime::from(now)),
                    Some(SurrealDatetime::from(now + Duration::days(14))),
                )
            } else {
                (None, None)
            };

            // Atomic CAS: only update if status still matches expected.
            // When not transitioning to trial, leave trial_* columns unchanged
            // (omit them from SET). When transitioning to trial, set dates.
            let mut result = if status == ApplicationStatus::Trial {
                self.client
                    .query(
                        "UPDATE $rid SET \
                            status = $new_status, \
                            reviewed_by = $reviewed_by, \
                            review_notes = $notes, \
                            updated_at = $now, \
                            trial_started_at = $trial_start, \
                            trial_ends_at = $trial_end \
                         WHERE status = $expected \
                         RETURN AFTER",
                    )
                    .bind(("rid", rid))
                    .bind(("new_status", status.to_string()))
                    .bind(("reviewed_by", reviewed_by.to_string()))
                    .bind(("notes", review_notes.map(|s| s.to_string())))
                    .bind(("now", SurrealDatetime::from(now)))
                    .bind(("trial_start", trial_started))
                    .bind(("trial_end", trial_ends))
                    .bind(("expected", expected_status.to_string()))
                    .await?
            } else {
                self.client
                    .query(
                        "UPDATE $rid SET \
                            status = $new_status, \
                            reviewed_by = $reviewed_by, \
                            review_notes = $notes, \
                            updated_at = $now \
                         WHERE status = $expected \
                         RETURN AFTER",
                    )
                    .bind(("rid", rid))
                    .bind(("new_status", status.to_string()))
                    .bind(("reviewed_by", reviewed_by.to_string()))
                    .bind(("notes", review_notes.map(|s| s.to_string())))
                    .bind(("now", SurrealDatetime::from(now)))
                    .bind(("expected", expected_status.to_string()))
                    .await?
            };

            let updated: Option<DbApplication> = result.take(0)?;
            if let Some(row) = updated {
                return Ok(db_to_application(row));
            }

            // Distinguish missing vs concurrent conflict.
            let existing: Option<DbApplication> =
                self.client.select(("application", id)).await?;
            match existing {
                None => Err(crate::DbError::NotFound(format!(
                    "Application {id} not found"
                ))),
                Some(row) => {
                    let current = parse_status(&row.status);
                    Err(crate::DbError::Conflict(format!(
                        "Application status changed concurrently (expected {expected_status}, found {current})"
                    )))
                }
            }
        })
        .await
    }

    /// List applications with trials expiring within the given number of days.
    pub async fn list_expiring_trials(&self, days: i64) -> DbResult<Vec<Application>> {
        with_timeout(async {
            let deadline = SurrealDatetime::from(Utc::now() + Duration::days(days));
            let mut result = self
                .client
                .query(
                    "SELECT * FROM application WHERE status = 'trial' AND trial_ends_at != NONE AND trial_ends_at <= $deadline ORDER BY trial_ends_at ASC LIMIT 200"
                )
                .bind(("deadline", deadline))
                .await?;
            let apps: Vec<DbApplication> = result.take(0)?;
            Ok(apps.into_iter().map(db_to_application).collect())
        })
        .await
    }
}
