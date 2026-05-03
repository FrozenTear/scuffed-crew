use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, ForumReply, ForumThread, NostrKeyMode};

use scuffed_chat::nostr::events::EventBuilder;
use scuffed_chat::nostr::relay::publish_event_oneshot;

use crate::extractors::{OfficerUser, OrgMember};
use crate::routes::audit_log::audit;
use crate::state::AppState;

// ─── Request/Response types ─────────────────────────────────

#[derive(Deserialize)]
pub struct ListThreadsQuery {
    pub category: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_limit() -> u32 {
    25
}

#[derive(Serialize)]
pub struct ThreadListResponse {
    pub threads: Vec<ThreadWithReplyCount>,
    pub total: usize,
}

#[derive(Serialize)]
pub struct ThreadWithReplyCount {
    #[serde(flatten)]
    pub thread: ForumThread,
    pub reply_count: u64,
}

#[derive(Serialize)]
pub struct ThreadDetailResponse {
    pub thread: ForumThread,
    pub replies: Vec<ForumReply>,
    pub reply_count: u64,
}

#[derive(Deserialize)]
pub struct CreateThreadRequest {
    pub title: String,
    pub content: String,
    #[serde(default = "default_category")]
    pub category: String,
}

fn default_category() -> String {
    "general".to_string()
}

#[derive(Deserialize)]
pub struct CreateReplyRequest {
    pub content: String,
}

#[derive(Deserialize)]
pub struct PinRequest {
    pub pinned: bool,
}

#[derive(Deserialize)]
pub struct LockRequest {
    pub locked: bool,
}

#[derive(Deserialize)]
pub struct ListRepliesQuery {
    #[serde(default = "default_reply_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_reply_limit() -> u32 {
    50
}

// ─── Handlers ───────────────────────────────────────────────

/// GET /api/forum/threads -- list threads (public)
pub async fn list_threads(
    State(state): State<AppState>,
    Query(query): Query<ListThreadsQuery>,
) -> Result<Json<ThreadListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let threads = state
        .db
        .list_forum_threads(query.category.as_deref(), query.limit, query.offset)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    let mut items = Vec::with_capacity(threads.len());
    for thread in threads {
        let reply_count = state
            .db
            .count_forum_replies(&thread.id)
            .await
            .unwrap_or(0);
        items.push(ThreadWithReplyCount {
            thread,
            reply_count,
        });
    }

    let total = items.len();
    Ok(Json(ThreadListResponse {
        threads: items,
        total,
    }))
}

/// GET /api/forum/threads/:id -- get thread + replies
pub async fn get_thread(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ListRepliesQuery>,
) -> Result<Json<ThreadDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let thread = state.db.get_forum_thread(&id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    let replies = state
        .db
        .list_forum_replies(&id, query.limit, query.offset)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    let reply_count = state.db.count_forum_replies(&id).await.unwrap_or(0);

    Ok(Json(ThreadDetailResponse {
        thread,
        replies,
        reply_count,
    }))
}

/// POST /api/forum/threads -- create thread (member auth)
pub async fn create_thread(
    State(state): State<AppState>,
    member: OrgMember,
    Json(body): Json<CreateThreadRequest>,
) -> Result<(StatusCode, Json<ForumThread>), (StatusCode, Json<ErrorResponse>)> {
    if body.title.trim().is_empty() || body.content.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Title and content are required".into(),
            }),
        ));
    }

    let thread = state
        .db
        .create_forum_thread(&body.title, &body.category, &member.member.id, &body.content)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    maybe_publish_thread_to_relay(&state, &member.member, &thread, &body.category).await;

    Ok((StatusCode::CREATED, Json(thread)))
}

/// POST /api/forum/threads/:id/replies -- reply to thread (member auth)
pub async fn create_reply(
    State(state): State<AppState>,
    member: OrgMember,
    Path(id): Path<String>,
    Json(body): Json<CreateReplyRequest>,
) -> Result<(StatusCode, Json<ForumReply>), (StatusCode, Json<ErrorResponse>)> {
    if body.content.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Content is required".into(),
            }),
        ));
    }

    // Verify thread exists and is not locked
    let thread = state.db.get_forum_thread(&id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    if thread.locked {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Thread is locked".into(),
            }),
        ));
    }

    if !thread.is_active {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Thread not found".into(),
            }),
        ));
    }

    let reply = state
        .db
        .create_forum_reply(&id, &member.member.id, &body.content)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    maybe_publish_reply_to_relay(&state, &member.member, &reply, thread.nostr_event_id.as_deref())
        .await;

    Ok((StatusCode::CREATED, Json(reply)))
}

/// PATCH /api/forum/threads/:id/pin -- pin/unpin thread (officer+)
pub async fn pin_thread(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
    Json(body): Json<PinRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .pin_forum_thread(&id, body.pinned)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    let detail = if body.pinned { "Pinned" } else { "Unpinned" };
    audit(
        &state.db,
        &officer.member.id,
        AuditAction::PinnedForumThread,
        AuditTargetType::ForumThread,
        &id,
        Some(detail),
    )
    .await;

    Ok(StatusCode::OK)
}

/// PATCH /api/forum/threads/:id/lock -- lock/unlock thread (officer+)
pub async fn lock_thread(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
    Json(body): Json<LockRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .lock_forum_thread(&id, body.locked)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    let detail = if body.locked { "Locked" } else { "Unlocked" };
    audit(
        &state.db,
        &officer.member.id,
        AuditAction::LockedForumThread,
        AuditTargetType::ForumThread,
        &id,
        Some(detail),
    )
    .await;

    Ok(StatusCode::OK)
}

// ─── Nostr dual-publish helpers ────────────────────────────

async fn is_nostr_forum(state: &AppState) -> bool {
    state
        .db
        .get_settings()
        .await
        .map(|s| s.forum_backend == "nostr")
        .unwrap_or(false)
}

async fn maybe_publish_thread_to_relay(
    state: &AppState,
    member: &scuffed_db::Member,
    thread: &ForumThread,
    category: &str,
) {
    if !is_nostr_forum(state).await {
        return;
    }
    let relay_url = match state.relay_url.clone() {
        Some(url) => url,
        None => return,
    };
    if member.nostr_key_mode != Some(NostrKeyMode::ServerManaged) {
        return;
    }
    let secret_hex = match state.db.get_nostr_secret_key(&member.id).await {
        Ok(Some(key)) => key,
        _ => return,
    };
    let keys = match EventBuilder::keys_from_hex(&secret_hex) {
        Ok(k) => k,
        Err(_) => return,
    };

    let content = format!("{}\n\n{}", thread.title, thread.content);
    let hashtags = vec![format!("forum-{category}")];
    let event = match EventBuilder::build_community_post(
        &keys, &content, &hashtags, None, None, None, None,
    ) {
        Ok(e) => e,
        Err(_) => return,
    };

    let relay_event = EventBuilder::to_relay_event(&event);
    let nostr_event_id = relay_event.id.clone();
    let thread_id = thread.id.clone();
    let db = state.db.clone();
    tokio::spawn(async move {
        if let Err(e) = publish_event_oneshot(&relay_url, relay_event).await {
            tracing::error!("Failed to publish forum thread {thread_id} to relay: {e}");
        } else {
            tracing::info!("Dual-published forum thread {thread_id} to Nostr relay");
            if let Err(e) = db.update_thread_nostr_event_id(&thread_id, &nostr_event_id).await {
                tracing::error!("Failed to store nostr_event_id for thread {thread_id}: {e}");
            }
        }
    });
}

async fn maybe_publish_reply_to_relay(
    state: &AppState,
    member: &scuffed_db::Member,
    reply: &ForumReply,
    thread_nostr_event_id: Option<&str>,
) {
    if !is_nostr_forum(state).await {
        return;
    }
    let relay_url = match state.relay_url.clone() {
        Some(url) => url,
        None => return,
    };
    if member.nostr_key_mode != Some(NostrKeyMode::ServerManaged) {
        return;
    }
    let secret_hex = match state.db.get_nostr_secret_key(&member.id).await {
        Ok(Some(key)) => key,
        _ => return,
    };
    let keys = match EventBuilder::keys_from_hex(&secret_hex) {
        Ok(k) => k,
        Err(_) => return,
    };

    let event = match EventBuilder::build_community_post(
        &keys,
        &reply.content,
        &[],
        None,
        thread_nostr_event_id,
        thread_nostr_event_id,
        None,
    ) {
        Ok(e) => e,
        Err(_) => return,
    };

    let relay_event = EventBuilder::to_relay_event(&event);
    let reply_id = reply.id.clone();
    tokio::spawn(async move {
        if let Err(e) = publish_event_oneshot(&relay_url, relay_event).await {
            tracing::error!("Failed to publish forum reply {reply_id} to relay: {e}");
        } else {
            tracing::info!("Dual-published forum reply {reply_id} to Nostr relay");
        }
    });
}
