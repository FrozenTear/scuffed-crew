use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::{NewStatReport, StatReport};
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbStatReport {
    #[surreal(default)]
    #[serde(default)]
    id: Option<RecordId>,
    member_id: String,
    created_at: SurrealDatetime,
    reason_category: String,
    reason_text: String,
    app_version: String,
    recognizer_matcher: String,
    recognizer_ocr: String,
    training_consent: bool,
    own_name_included: bool,
    glyphs_included: bool,
    size_bytes: i64,
    zip_sha256: String,
    #[serde(default)]
    #[surreal(default)]
    expires_at: Option<SurrealDatetime>,
}

fn db_to_report(db: DbStatReport) -> StatReport {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    StatReport {
        id,
        member_id: db.member_id,
        created_at: db.created_at.into(),
        reason_category: db.reason_category,
        reason_text: db.reason_text,
        app_version: db.app_version,
        recognizer_matcher: db.recognizer_matcher,
        recognizer_ocr: db.recognizer_ocr,
        training_consent: db.training_consent,
        own_name_included: db.own_name_included,
        glyphs_included: db.glyphs_included,
        size_bytes: u64::try_from(db.size_bytes).unwrap_or(0),
        zip_sha256: db.zip_sha256,
        expires_at: db.expires_at.map(Into::into),
    }
}

fn to_db(report: &NewStatReport) -> Result<DbStatReport, crate::DbError> {
    let size_bytes = i64::try_from(report.size_bytes).map_err(|_| {
        crate::DbError::Config("report size does not fit a database integer".into())
    })?;
    Ok(DbStatReport {
        id: None,
        member_id: report.member_id.clone(),
        created_at: SurrealDatetime::from(report.created_at),
        reason_category: report.reason_category.clone(),
        reason_text: report.reason_text.clone(),
        app_version: report.app_version.clone(),
        recognizer_matcher: report.recognizer_matcher.clone(),
        recognizer_ocr: report.recognizer_ocr.clone(),
        training_consent: report.training_consent,
        own_name_included: report.own_name_included,
        glyphs_included: report.glyphs_included,
        size_bytes,
        zip_sha256: report.zip_sha256.clone(),
        expires_at: report.expires_at.map(SurrealDatetime::from),
    })
}

impl Database {
    pub async fn insert_stat_report(&self, report: &NewStatReport) -> DbResult<StatReport> {
        let id = report.id.clone();
        let row = to_db(report)?;
        with_timeout(async {
            let created: Option<DbStatReport> = self
                .client
                .create(("stat_report", id.as_str()))
                .content(row)
                .await?;
            Ok(db_to_report(created.ok_or_else(|| {
                crate::DbError::NotFound("failed to create stat report".into())
            })?))
        })
        .await
    }

    pub async fn get_stat_report(&self, id: &str) -> DbResult<Option<StatReport>> {
        let id = id.to_string();
        with_timeout(async {
            let row: Option<DbStatReport> =
                self.client.select(("stat_report", id.as_str())).await?;
            Ok(row.map(db_to_report))
        })
        .await
    }

    pub async fn count_stat_reports_since(
        &self,
        member_id: &str,
        start: DateTime<Utc>,
    ) -> DbResult<u64> {
        let member_id = member_id.to_string();
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct CountResult {
                count: u64,
            }
            let mut result = self
                .client
                .query(
                    "SELECT count() AS count FROM stat_report \
                     WHERE member_id = $member_id AND created_at >= $start GROUP ALL",
                )
                .bind(("member_id", member_id))
                .bind(("start", SurrealDatetime::from(start)))
                .await?;
            let counts: Vec<CountResult> = result.take(0)?;
            Ok(counts.first().map(|c| c.count).unwrap_or(0))
        })
        .await
    }

    pub async fn list_stat_reports_for_member(&self, member_id: &str) -> DbResult<Vec<StatReport>> {
        let member_id = member_id.to_string();
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT * FROM stat_report WHERE member_id = $member_id \
                     ORDER BY created_at DESC LIMIT 1000",
                )
                .bind(("member_id", member_id))
                .await?;
            let rows: Vec<DbStatReport> = result.take(0)?;
            Ok(rows.into_iter().map(db_to_report).collect())
        })
        .await
    }

    pub async fn list_stat_reports(&self) -> DbResult<Vec<StatReport>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM stat_report ORDER BY created_at DESC LIMIT 1000")
                .await?;
            let rows: Vec<DbStatReport> = result.take(0)?;
            Ok(rows.into_iter().map(db_to_report).collect())
        })
        .await
    }

    pub async fn list_stat_report_ids(&self) -> DbResult<Vec<String>> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct IdRow {
                id: Option<RecordId>,
            }
            let mut result = self.client.query("SELECT id FROM stat_report").await?;
            let rows: Vec<IdRow> = result.take(0)?;
            Ok(rows
                .into_iter()
                .filter_map(|row| row.id.map(|rid| crate::record_id_key_to_string(rid.key)))
                .collect())
        })
        .await
    }

    /// Reports whose retention deadline has passed. Training rows have no deadline.
    pub async fn list_expired_stat_reports(&self, now: DateTime<Utc>) -> DbResult<Vec<StatReport>> {
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT * FROM stat_report \
                     WHERE expires_at != NONE AND expires_at <= $now LIMIT 1000",
                )
                .bind(("now", SurrealDatetime::from(now)))
                .await?;
            let rows: Vec<DbStatReport> = result.take(0)?;
            Ok(rows.into_iter().map(db_to_report).collect())
        })
        .await
    }

    pub async fn delete_stat_report(&self, id: &str) -> DbResult<bool> {
        let id = id.to_string();
        with_timeout(async {
            let removed: Option<DbStatReport> =
                self.client.delete(("stat_report", id.as_str())).await?;
            Ok(removed.is_some())
        })
        .await
    }

    /// Clear training consent and set expiry to the original received time plus 30 days.
    ///
    /// Returns the updated row when this call won the compare-and-swap. `Ok(None)`
    /// means the row was already not in training (or was deleted).
    pub async fn withdraw_stat_report_training(
        &self,
        id: &str,
        expires_at: DateTime<Utc>,
    ) -> DbResult<Option<StatReport>> {
        let id_owned = id.to_string();
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "UPDATE $rid SET training_consent = false, expires_at = $expires \
                     WHERE training_consent = true",
                )
                .bind(("rid", RecordId::new("stat_report", id_owned.as_str())))
                .bind(("expires", SurrealDatetime::from(expires_at)))
                .await?;
            let rows: Vec<DbStatReport> = result.take(0)?;
            Ok(rows.into_iter().next().map(db_to_report))
        })
        .await
    }

    /// Move the stored clock. Used by retention tests. Not an HTTP route.
    pub async fn set_stat_report_clock(
        &self,
        id: &str,
        created_at: DateTime<Utc>,
        expires_at: Option<DateTime<Utc>>,
    ) -> DbResult<()> {
        let id_owned = id.to_string();
        with_timeout(async {
            self.client
                .query("UPDATE $rid SET created_at = $created, expires_at = $expires")
                .bind(("rid", RecordId::new("stat_report", id_owned.as_str())))
                .bind(("created", SurrealDatetime::from(created_at)))
                .bind(("expires", expires_at.map(SurrealDatetime::from)))
                .await?
                .check()?;
            Ok(())
        })
        .await
    }
}

/// Received time plus the 30 day retention window.
pub fn retention_deadline(created_at: DateTime<Utc>) -> DateTime<Utc> {
    created_at + Duration::days(scuffed_types::RETENTION_DAYS)
}

/// Start of the UTC day that contains `now`.
pub fn utc_day_start(now: DateTime<Utc>) -> DateTime<Utc> {
    now.date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("midnight")
        .and_utc()
}
