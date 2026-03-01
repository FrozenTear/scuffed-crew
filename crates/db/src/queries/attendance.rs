use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::sql::Datetime as SurrealDatetime;
use surrealdb::sql::Thing;

use crate::types::{AttendanceStats, AttendanceStatus, EventAttendance};
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DbEventAttendance {
    #[serde(skip_serializing)]
    #[allow(dead_code)]
    id: Option<Thing>,
    member_id: String,
    event_id: String,
    occurrence_date: SurrealDatetime,
    status: String,
    marked_by: String,
    marked_at: SurrealDatetime,
}

fn parse_attendance_status(s: &str) -> AttendanceStatus {
    match s {
        "attended" => AttendanceStatus::Attended,
        "excused" => AttendanceStatus::Excused,
        _ => AttendanceStatus::NoShow,
    }
}

fn db_to_attendance(db: DbEventAttendance) -> EventAttendance {
    let id = db
        .id
        .map(|t| t.id.to_raw())
        .unwrap_or_else(|| "unknown".to_string());
    EventAttendance {
        id,
        member_id: db.member_id,
        event_id: db.event_id,
        occurrence_date: db.occurrence_date.into(),
        status: parse_attendance_status(&db.status),
        marked_by: db.marked_by,
        marked_at: db.marked_at.into(),
    }
}

impl Database {
    /// Mark attendance for a member at an event occurrence (upsert).
    pub async fn mark_attendance(
        &self,
        member_id: &str,
        event_id: &str,
        occurrence_date: DateTime<Utc>,
        status: AttendanceStatus,
        marked_by: &str,
    ) -> DbResult<EventAttendance> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let occ = SurrealDatetime::from(occurrence_date);
            let mut result = self
                .client
                .query(
                    r#"
                    LET $existing = (SELECT * FROM event_attendance WHERE member_id = $mid AND event_id = $eid AND occurrence_date = $occ LIMIT 1);
                    IF array::len($existing) > 0 {
                        UPDATE $existing[0].id SET
                            status = $st,
                            marked_by = $mb,
                            marked_at = $mat
                        ;
                    } ELSE {
                        CREATE event_attendance SET
                            member_id = $mid,
                            event_id = $eid,
                            occurrence_date = $occ,
                            status = $st,
                            marked_by = $mb,
                            marked_at = $mat
                        ;
                    };
                    "#,
                )
                .bind(("mid", member_id.to_string()))
                .bind(("eid", event_id.to_string()))
                .bind(("occ", occ))
                .bind(("st", status.to_string()))
                .bind(("mb", marked_by.to_string()))
                .bind(("mat", now))
                .await?;

            let records: Vec<DbEventAttendance> = result.take(1)?;
            records
                .into_iter()
                .next()
                .map(db_to_attendance)
                .ok_or_else(|| crate::DbError::NotFound("Failed to mark attendance".into()))
        })
        .await
    }

    /// List attendance records for an event occurrence.
    pub async fn list_event_attendance(
        &self,
        event_id: &str,
        occurrence_date: DateTime<Utc>,
    ) -> DbResult<Vec<EventAttendance>> {
        with_timeout(async {
            let occ = SurrealDatetime::from(occurrence_date);
            let mut result = self
                .client
                .query("SELECT * FROM event_attendance WHERE event_id = $eid AND occurrence_date = $occ ORDER BY member_id ASC")
                .bind(("eid", event_id.to_string()))
                .bind(("occ", occ))
                .await?;
            let records: Vec<DbEventAttendance> = result.take(0)?;
            Ok(records.into_iter().map(db_to_attendance).collect())
        })
        .await
    }

    /// Get attendance stats for a member.
    pub async fn get_member_attendance_stats(
        &self,
        member_id: &str,
    ) -> DbResult<AttendanceStats> {
        with_timeout(async {
            #[derive(Deserialize)]
            struct CountResult {
                count: u32,
            }

            let mut attended_result = self
                .client
                .query("SELECT count() FROM event_attendance WHERE member_id = $mid AND status = 'attended' GROUP ALL")
                .bind(("mid", member_id.to_string()))
                .await?;
            let attended: Vec<CountResult> = attended_result.take(0)?;

            let mut no_show_result = self
                .client
                .query("SELECT count() FROM event_attendance WHERE member_id = $mid AND status = 'no_show' GROUP ALL")
                .bind(("mid", member_id.to_string()))
                .await?;
            let no_show: Vec<CountResult> = no_show_result.take(0)?;

            let mut excused_result = self
                .client
                .query("SELECT count() FROM event_attendance WHERE member_id = $mid AND status = 'excused' GROUP ALL")
                .bind(("mid", member_id.to_string()))
                .await?;
            let excused: Vec<CountResult> = excused_result.take(0)?;

            let a = attended.first().map(|c| c.count).unwrap_or(0);
            let n = no_show.first().map(|c| c.count).unwrap_or(0);
            let e = excused.first().map(|c| c.count).unwrap_or(0);

            Ok(AttendanceStats {
                member_id: member_id.to_string(),
                attended: a,
                no_show: n,
                excused: e,
                total: a + n + e,
            })
        })
        .await
    }

    /// Get member attendance history.
    pub async fn list_member_attendance(
        &self,
        member_id: &str,
    ) -> DbResult<Vec<EventAttendance>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM event_attendance WHERE member_id = $mid ORDER BY occurrence_date DESC")
                .bind(("mid", member_id.to_string()))
                .await?;
            let records: Vec<DbEventAttendance> = result.take(0)?;
            Ok(records.into_iter().map(db_to_attendance).collect())
        })
        .await
    }
}
