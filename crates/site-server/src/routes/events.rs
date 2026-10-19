use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde::de::{self, Deserializer, Visitor};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, Event};
use scuffed_types::api::{CursorResponse, PaginationParams};

use crate::extractors::{OfficerUser, OptionalOrgMember};
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// GET /api/events — list active events (cursor-paginated).
/// Anonymous: `is_public` only. Org members: all active events.
pub async fn list_events(
    State(state): State<AppState>,
    OptionalOrgMember(member): OptionalOrgMember,
    axum::extract::Query(pagination): axum::extract::Query<PaginationParams>,
) -> Result<Json<CursorResponse<Event>>, (StatusCode, Json<ErrorResponse>)> {
    let (limit, offset) = pagination.resolve();
    let only_public = member.is_none();
    let items = state
        .db
        .list_events_paginated(limit, offset, only_public)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
    Ok(Json(CursorResponse::from_oversized(items, limit, offset)))
}

/// JSON integer for `day_of_week`, including values that do not fit in a `u8`.
///
/// The handler rejects anything outside 0..=6 with 400. A non-integer still
/// fails deserialization, which Axum reports as 422.
#[derive(Debug, Clone, Copy)]
pub struct DayOfWeekInput(i128);

impl<'de> Deserialize<'de> for DayOfWeekInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DayVisitor;

        impl Visitor<'_> for DayVisitor {
            type Value = DayOfWeekInput;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an integer day_of_week")
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(DayOfWeekInput(i128::from(value)))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(DayOfWeekInput(i128::from(value)))
            }
        }

        deserializer.deserialize_any(DayVisitor)
    }
}

#[derive(Deserialize)]
pub struct CreateEventRequest {
    pub title: String,
    pub day_of_week: DayOfWeekInput,
    pub time: String,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    #[serde(default = "default_duration")]
    pub duration_minutes: u32,
    #[serde(default = "default_true")]
    pub is_recurring: bool,
    pub team_id: Option<String>,
    /// Default false — practice slots stay private until published.
    #[serde(default)]
    pub is_public: bool,
}

fn default_timezone() -> String {
    "Europe/Berlin".to_string()
}
fn default_duration() -> u32 {
    120
}
fn default_true() -> bool {
    true
}

fn bad_request(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: msg.to_string(),
        }),
    )
}

/// Reject CR/LF and other controls so a stored event cannot be turned into
/// extra iCalendar lines later. Does not echo the value.
fn reject_controls(field: &str, value: &str) -> Result<(), String> {
    if value
        .chars()
        .any(|c| c.is_control() || c == '\u{2028}' || c == '\u{2029}')
    {
        return Err(format!(
            "{field} must not contain line breaks or control characters"
        ));
    }
    Ok(())
}

/// TZID is interpolated into `DTSTART;TZID=…`. Only an IANA-style token is safe.
fn validate_timezone(tz: &str) -> Result<(), String> {
    if tz.is_empty()
        || tz.len() > 64
        || !tz
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'))
    {
        return Err("invalid timezone".into());
    }
    Ok(())
}

fn validate_event_text(
    title: &str,
    time: &str,
    timezone: &str,
    team_id: Option<&str>,
) -> Result<(), String> {
    reject_controls("title", title)?;
    reject_controls("time", time)?;
    validate_timezone(timezone)?;
    if let Some(team_id) = team_id {
        reject_controls("team", team_id)?;
    }
    Ok(())
}

fn schedule_error(time: &str, duration_minutes: u32) -> Result<(), String> {
    crate::calendar::check_event_schedule(time, duration_minutes)
        .map_err(|err| err.message().to_string())
}

/// When only the clock or only the duration is being changed, check it against
/// the row that is already stored. A missing row is left for the update call.
async fn merged_schedule(
    state: &AppState,
    id: &str,
    time: Option<&str>,
    duration_minutes: Option<u32>,
) -> Result<Option<(String, u32)>, (StatusCode, Json<ErrorResponse>)> {
    match (time, duration_minutes) {
        (None, None) => Ok(None),
        (Some(time), Some(duration_minutes)) => Ok(Some((time.to_string(), duration_minutes))),
        (time, duration_minutes) => match state.db.get_event(id).await {
            Ok(Some(existing)) => Ok(Some((
                time.unwrap_or(existing.time.as_str()).to_string(),
                duration_minutes.unwrap_or(existing.duration_minutes),
            ))),
            Ok(None) => Ok(None),
            Err(err) => {
                tracing::error!(error = %err, "load event before schedule check");
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Internal error".into(),
                    }),
                ))
            }
        },
    }
}

/// POST /api/events — create event (officer+)
pub async fn create_event(
    State(state): State<AppState>,
    officer: OfficerUser,
    Json(body): Json<CreateEventRequest>,
) -> Result<(StatusCode, Json<Event>), (StatusCode, Json<ErrorResponse>)> {
    validate_event_text(
        &body.title,
        &body.time,
        &body.timezone,
        body.team_id.as_deref(),
    )
    .map_err(|msg| bad_request(&msg))?;
    schedule_error(&body.time, body.duration_minutes).map_err(|msg| bad_request(&msg))?;
    let day_of_week =
        crate::calendar::check_day_of_week(body.day_of_week.0).map_err(bad_request)?;

    let event = state
        .db
        .create_event(
            &body.title,
            day_of_week,
            &body.time,
            &body.timezone,
            body.duration_minutes,
            body.is_recurring,
            body.team_id.as_deref(),
            &officer.member.id,
            body.is_public,
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
    audit(
        &state.db,
        &officer.member.id,
        AuditAction::CreatedEvent,
        AuditTargetType::Event,
        &event.id,
        Some(&format!("Created event: {}", event.title)),
    )
    .await;

    Ok((StatusCode::CREATED, Json(event)))
}

#[derive(Deserialize)]
pub struct UpdateEventRequest {
    pub title: Option<String>,
    pub day_of_week: Option<DayOfWeekInput>,
    pub time: Option<String>,
    pub timezone: Option<String>,
    pub duration_minutes: Option<u32>,
    pub is_recurring: Option<bool>,
    pub team_id: Option<Option<String>>,
    pub is_public: Option<bool>,
}

/// PUT /api/events/:id — update event (officer+)
pub async fn update_event(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateEventRequest>,
) -> Result<Json<Event>, (StatusCode, Json<ErrorResponse>)> {
    let day_of_week = match body.day_of_week {
        Some(day) => Some(crate::calendar::check_day_of_week(day.0).map_err(bad_request)?),
        None => None,
    };
    if let Some(title) = body.title.as_deref() {
        reject_controls("title", title).map_err(|msg| bad_request(&msg))?;
    }
    if let Some(time) = body.time.as_deref() {
        reject_controls("time", time).map_err(|msg| bad_request(&msg))?;
        if crate::calendar::parse_hhmm(time).is_none() {
            return Err(bad_request(crate::calendar::ScheduleError::Time.message()));
        }
    }
    if let Some(duration_minutes) = body.duration_minutes {
        if duration_minutes == 0 {
            return Err(bad_request(
                crate::calendar::ScheduleError::EndsBeforeStart.message(),
            ));
        }
        if duration_minutes > crate::calendar::MAX_EVENT_DURATION_MINUTES {
            return Err(bad_request(
                crate::calendar::ScheduleError::Duration.message(),
            ));
        }
    }
    if let Some(timezone) = body.timezone.as_deref() {
        validate_timezone(timezone).map_err(|msg| bad_request(&msg))?;
    }
    if let Some(Some(team_id)) = body.team_id.as_ref() {
        reject_controls("team", team_id).map_err(|msg| bad_request(&msg))?;
    }
    if let Some((time, duration_minutes)) =
        merged_schedule(&state, &id, body.time.as_deref(), body.duration_minutes).await?
    {
        schedule_error(&time, duration_minutes).map_err(|msg| bad_request(&msg))?;
    }

    let event = state
        .db
        .update_event(
            &id,
            body.title.as_deref(),
            day_of_week,
            body.time.as_deref(),
            body.timezone.as_deref(),
            body.duration_minutes,
            body.is_recurring,
            body.team_id.as_ref().map(|t| t.as_deref()),
            body.is_public,
        )
        .await
        .map_err(|err| match err {
            scuffed_db::DbError::NotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Event not found".into(),
                }),
            ),
            other => {
                tracing::error!(error = %other, "update event failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Internal error".into(),
                    }),
                )
            }
        })?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::UpdatedEvent,
        AuditTargetType::Event,
        &id,
        None,
    )
    .await;

    Ok(Json(event))
}

/// DELETE /api/events/:id — deactivate event (officer+)
pub async fn delete_event(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state.db.deactivate_event(&id).await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::DeletedEvent,
        AuditTargetType::Event,
        &id,
        None,
    )
    .await;

    Ok(StatusCode::OK)
}
