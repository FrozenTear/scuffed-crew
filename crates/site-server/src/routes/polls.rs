use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, Poll, PollResults};

use crate::extractors::{OfficerUser, OrgMember};
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// GET /api/polls — list active polls with results and viewer's votes (member)
pub async fn list_polls(
    State(state): State<AppState>,
    member: OrgMember,
) -> Result<Json<Vec<PollResults>>, (StatusCode, Json<ErrorResponse>)> {
    let polls = state.db.list_polls().await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;
    let mut results = Vec::with_capacity(polls.len());
    for poll in &polls {
        let r = state
            .db
            .get_poll_results(&poll.id, Some(&member.member.id))
            .await
            .map_err(|_e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Internal error".into(),
                    }),
                )
            })?;
        results.push(r);
    }
    Ok(Json(results))
}

/// GET /api/polls/:id — get poll with results + viewer's votes (authed)
pub async fn get_poll(
    State(state): State<AppState>,
    member: OrgMember,
    Path(id): Path<String>,
) -> Result<Json<PollResults>, (StatusCode, Json<ErrorResponse>)> {
    let results = state
        .db
        .get_poll_results(&id, Some(&member.member.id))
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
    Ok(Json(results))
}

#[derive(Deserialize)]
pub struct CreatePollRequest {
    pub title: String,
    pub description: Option<String>,
    pub options: Vec<String>,
    pub close_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub allow_multiple: bool,
}

/// POST /api/polls — create poll (officer+)
pub async fn create_poll(
    State(state): State<AppState>,
    officer: OfficerUser,
    Json(body): Json<CreatePollRequest>,
) -> Result<(StatusCode, Json<Poll>), (StatusCode, Json<ErrorResponse>)> {
    if body.options.len() < 2 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Poll must have at least 2 options".into(),
            }),
        ));
    }

    let poll = state
        .db
        .create_poll(
            &body.title,
            body.description.as_deref(),
            &body.options,
            body.close_at,
            body.allow_multiple,
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

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::CreatedPoll,
        AuditTargetType::Poll,
        &poll.id,
        Some(&format!("Created poll: {}", poll.title)),
    )
    .await;

    Ok((StatusCode::CREATED, Json(poll)))
}

#[derive(Deserialize)]
pub struct VoteRequest {
    pub option_index: u32,
}

/// POST /api/polls/:id/vote — cast vote (member)
pub async fn vote_poll(
    State(state): State<AppState>,
    member: OrgMember,
    Path(id): Path<String>,
    Json(body): Json<VoteRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let poll = state.db.get_poll(&id).await.map_err(|_e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    if body.option_index as usize >= poll.options.len() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid option index".into(),
            }),
        ));
    }

    state
        .db
        .vote_poll(&id, &member.member.id, body.option_index)
        .await
        .map_err(|e| match e {
            // Poll vanished between the load above and the write.
            scuffed_db::DbError::NotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Poll not found".into(),
                }),
            ),
            // Closed poll, duplicate same-option vote, or single-choice replace
            // race — all surface as a 409 with a caller-safe message.
            scuffed_db::DbError::Conflict(msg) => {
                (StatusCode::CONFLICT, Json(ErrorResponse { error: msg }))
            }
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            ),
        })?;

    Ok(StatusCode::OK)
}

/// DELETE /api/polls/:id/vote/:option_index — remove vote (member)
pub async fn unvote_poll(
    State(state): State<AppState>,
    member: OrgMember,
    Path((id, option_index)): Path<(String, u32)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .unvote_poll(&id, &member.member.id, option_index)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
    Ok(StatusCode::OK)
}

/// DELETE /api/polls/:id — deactivate poll (officer+)
pub async fn deactivate_poll(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state.db.deactivate_poll(&id).await.map_err(|_e| {
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
        AuditAction::DeletedPoll,
        AuditTargetType::Poll,
        &id,
        Some("Deactivated poll"),
    )
    .await;

    Ok(StatusCode::OK)
}
