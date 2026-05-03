use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use scuffed_auth::server::AuthUser;
use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, Poll, PollResults};

use crate::extractors::{OfficerUser, OrgMember};
use crate::routes::audit_log::audit;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct PollDetailResponse {
    pub poll: Poll,
    pub results: PollResults,
    pub viewer_votes: Vec<u32>,
}

fn internal_error(err: impl ToString) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: err.to_string(),
        }),
    )
}

fn bad_request(message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
}

fn poll_not_found() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "Poll not found".to_string(),
        }),
    )
}

fn poll_is_closed(poll: &Poll) -> bool {
    poll.close_at.is_some_and(|close_at| close_at <= Utc::now())
}

/// GET /api/polls — list active polls (public)
pub async fn list_polls(
    State(state): State<AppState>,
) -> Result<Json<Vec<Poll>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .list_polls()
        .await
        .map(Json)
        .map_err(internal_error)
}

/// GET /api/polls/:id — get poll with aggregated results + viewer votes
pub async fn get_poll(
    State(state): State<AppState>,
    Path(id): Path<String>,
    viewer: Result<
        AuthUser<AppState>,
        <AuthUser<AppState> as axum::extract::FromRequestParts<AppState>>::Rejection,
    >,
) -> Result<Json<PollDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let poll = state
        .db
        .get_poll(&id)
        .await
        .map_err(internal_error)?
        .ok_or_else(poll_not_found)?;

    let results = state
        .db
        .get_poll_results(&id)
        .await
        .map_err(internal_error)?;

    let viewer_votes = if let Ok(auth_user) = viewer {
        if let Some(member) = state
            .db
            .get_member_by_user(&auth_user.id)
            .await
            .map_err(internal_error)?
        {
            state
                .db
                .get_member_poll_votes(&id, &member.id)
                .await
                .map_err(internal_error)?
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    Ok(Json(PollDetailResponse {
        poll,
        results,
        viewer_votes,
    }))
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreatePollRequest {
    pub title: String,
    pub description: Option<String>,
    pub options: Vec<String>,
    pub close_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub allow_multiple: bool,
}

/// POST /api/polls — create poll (officer+)
pub async fn create_poll(
    State(state): State<AppState>,
    officer: OfficerUser,
    Json(body): Json<CreatePollRequest>,
) -> Result<(StatusCode, Json<Poll>), (StatusCode, Json<ErrorResponse>)> {
    let title = body.title.trim().to_string();
    if title.is_empty() {
        return Err(bad_request("Title is required"));
    }

    let options: Vec<String> = body
        .options
        .iter()
        .map(|o| o.trim())
        .filter(|o| !o.is_empty())
        .map(|o| o.to_string())
        .collect();

    if options.len() < 2 {
        return Err(bad_request("At least two non-empty options are required"));
    }

    if let Some(close_at) = body.close_at.as_ref() {
        if *close_at <= Utc::now() {
            return Err(bad_request("close_at must be in the future"));
        }
    }

    let description = body.description.as_ref().and_then(|d| {
        let trimmed = d.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });

    let poll = state
        .db
        .create_poll(
            &title,
            description,
            options,
            body.close_at,
            body.allow_multiple,
            &officer.member.id,
        )
        .await
        .map_err(internal_error)?;

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

#[derive(Debug, Clone, Deserialize)]
pub struct VotePollRequest {
    pub option_index: u32,
}

/// POST /api/polls/:id/vote — cast vote (member)
pub async fn vote_poll(
    State(state): State<AppState>,
    member: OrgMember,
    Path(id): Path<String>,
    Json(body): Json<VotePollRequest>,
) -> Result<Json<PollDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let poll = state
        .db
        .get_poll(&id)
        .await
        .map_err(internal_error)?
        .ok_or_else(poll_not_found)?;

    if poll_is_closed(&poll) {
        return Err(bad_request("Poll is closed"));
    }

    if body.option_index as usize >= poll.options.len() {
        return Err(bad_request("Invalid option_index"));
    }

    state
        .db
        .vote_poll(
            &id,
            &member.member.id,
            body.option_index,
            poll.allow_multiple,
        )
        .await
        .map_err(internal_error)?;

    let results = state
        .db
        .get_poll_results(&id)
        .await
        .map_err(internal_error)?;
    let viewer_votes = state
        .db
        .get_member_poll_votes(&id, &member.member.id)
        .await
        .map_err(internal_error)?;

    Ok(Json(PollDetailResponse {
        poll,
        results,
        viewer_votes,
    }))
}

/// DELETE /api/polls/:id/vote/:option_index — remove vote (member)
pub async fn unvote_poll(
    State(state): State<AppState>,
    member: OrgMember,
    Path((id, option_index)): Path<(String, u32)>,
) -> Result<Json<PollDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let poll = state
        .db
        .get_poll(&id)
        .await
        .map_err(internal_error)?
        .ok_or_else(poll_not_found)?;

    if option_index as usize >= poll.options.len() {
        return Err(bad_request("Invalid option_index"));
    }

    state
        .db
        .unvote_poll(&id, &member.member.id, option_index)
        .await
        .map_err(internal_error)?;

    let results = state
        .db
        .get_poll_results(&id)
        .await
        .map_err(internal_error)?;
    let viewer_votes = state
        .db
        .get_member_poll_votes(&id, &member.member.id)
        .await
        .map_err(internal_error)?;

    Ok(Json(PollDetailResponse {
        poll,
        results,
        viewer_votes,
    }))
}

/// DELETE /api/polls/:id — deactivate poll (officer+)
pub async fn delete_poll(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .deactivate_poll(&id)
        .await
        .map_err(internal_error)?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::DeletedPoll,
        AuditTargetType::Poll,
        &id,
        None,
    )
    .await;

    Ok(StatusCode::OK)
}
