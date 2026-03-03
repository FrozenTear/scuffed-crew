use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use surrealdb_types::RecordId;
use surrealdb::types::Datetime as SurrealDatetime;
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
                created_at: now.clone(),
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

    pub async fn list_applications(&self) -> DbResult<Vec<Application>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM application ORDER BY created_at DESC")
                .await?;
            let apps: Vec<DbApplication> = result.take(0)?;
            Ok(apps.into_iter().map(db_to_application).collect())
        })
        .await
    }

    pub async fn get_application_by_user(
        &self,
        user_id: &str,
    ) -> DbResult<Option<Application>> {
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

    pub async fn update_application_status(
        &self,
        id: &str,
        status: ApplicationStatus,
        reviewed_by: &str,
        review_notes: Option<&str>,
    ) -> DbResult<Application> {
        with_timeout(async {
            let existing: Option<DbApplication> =
                self.client.select(("application", id)).await?;
            let mut db = existing.ok_or_else(|| {
                crate::DbError::NotFound(format!("Application {id} not found"))
            })?;

            db.status = status.to_string();
            db.reviewed_by = Some(reviewed_by.to_string());
            db.review_notes = review_notes.map(|s| s.to_string());
            db.updated_at = SurrealDatetime::from(Utc::now());

            // Auto-set trial dates when transitioning to trial status
            if status == ApplicationStatus::Trial {
                let now = Utc::now();
                db.trial_started_at = Some(SurrealDatetime::from(now));
                db.trial_ends_at = Some(SurrealDatetime::from(now + Duration::days(14)));
            }

            let updated: Option<DbApplication> = self
                .client
                .update(("application", id))
                .content(db)
                .await?;
            Ok(db_to_application(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Application {id} not found after update"))
            })?))
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
                    "SELECT * FROM application WHERE status = 'trial' AND trial_ends_at != NONE AND trial_ends_at <= $deadline ORDER BY trial_ends_at ASC"
                )
                .bind(("deadline", deadline))
                .await?;
            let apps: Vec<DbApplication> = result.take(0)?;
            Ok(apps.into_iter().map(db_to_application).collect())
        })
        .await
    }
}
