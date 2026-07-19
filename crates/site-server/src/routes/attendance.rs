use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{
    AttendanceStats, AttendanceStatus, AuditAction, AuditTargetType, EventAttendance,
};

use crate::extractors::{OfficerUser, OrgMember};
use crate::routes::audit_log::audit;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct AttendanceEntry {
    pub member_id: String,
    pub status: AttendanceStatus,
}

#[derive(Deserialize)]
pub struct BatchAttendanceRequest {
    pub occurrence_date: DateTime<Utc>,
    pub entries: Vec<AttendanceEntry>,
}

/// POST /api/events/:id/attendance — batch mark attendance (officer+)
pub async fn batch_mark_attendance(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(event_id): Path<String>,
    Json(body): Json<BatchAttendanceRequest>,
) -> Result<Json<Vec<EventAttendance>>, (StatusCode, Json<ErrorResponse>)> {
    let mut results = Vec::with_capacity(body.entries.len());

    for entry in body.entries {
        let record = state
            .db
            .mark_attendance(
                &entry.member_id,
                &event_id,
                body.occurrence_date,
                entry.status,
                &officer.member.id,
            )
            .await
            .map_err(|_e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Internal error".into(),
                    }),
                )
            })?;
        results.push(record);
    }

    let detail = format!("Marked attendance for {} member(s)", results.len());
    audit(
        &state.db,
        &officer.member.id,
        AuditAction::MarkedAttendance,
        AuditTargetType::Event,
        &event_id,
        Some(detail.as_str()),
    )
    .await;

    Ok(Json(results))
}

#[derive(Deserialize)]
pub struct AttendanceQuery {
    pub occurrence_date: DateTime<Utc>,
}

/// GET /api/events/:id/attendance — get attendance for an event occurrence (officer+)
pub async fn get_event_attendance(
    State(state): State<AppState>,
    _officer: OfficerUser,
    Path(event_id): Path<String>,
    Query(query): Query<AttendanceQuery>,
) -> Result<Json<Vec<EventAttendance>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .list_event_attendance(&event_id, query.occurrence_date)
        .await
        .map(Json)
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })
}

/// GET /api/members/:id/attendance — member attendance history (self or officer+)
pub async fn member_attendance(
    State(state): State<AppState>,
    caller: OrgMember,
    Path(member_id): Path<String>,
) -> Result<Json<Vec<EventAttendance>>, (StatusCode, Json<ErrorResponse>)> {
    let is_self = caller.member.id == member_id;
    let is_officer = caller
        .member
        .org_role
        .is_at_least(scuffed_db::OrgRole::Officer);

    if !is_self && !is_officer {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Can only view your own attendance".into(),
            }),
        ));
    }

    state
        .db
        .list_member_attendance(&member_id)
        .await
        .map(Json)
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })
}

/// GET /api/members/:id/attendance/stats — member attendance stats (self or officer+)
pub async fn member_attendance_stats(
    State(state): State<AppState>,
    caller: OrgMember,
    Path(member_id): Path<String>,
) -> Result<Json<AttendanceStats>, (StatusCode, Json<ErrorResponse>)> {
    let is_self = caller.member.id == member_id;
    let is_officer = caller
        .member
        .org_role
        .is_at_least(scuffed_db::OrgRole::Officer);

    if !is_self && !is_officer {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Can only view your own attendance stats".into(),
            }),
        ));
    }

    state
        .db
        .get_member_attendance_stats(&member_id)
        .await
        .map(Json)
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })
}
