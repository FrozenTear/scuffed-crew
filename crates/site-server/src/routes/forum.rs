use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{
    AuditAction, AuditTargetType, ForumBoard, ForumCategory, ForumCategoryNode, ForumReply,
    ForumThread, NostrKeyMode,
};
use zeroize::Zeroize;

use scuffed_chat::nostr::events::EventBuilder;
use scuffed_chat::nostr::relay::publish_event_oneshot;

use scuffed_db::OrgRole;

use crate::extractors::{OfficerUser, OptionalOrgMember, OrgMember};
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// Whether `role` meets board `min_role` (`admin`/`officer`/`member`/`recruit`).
/// Empty/unknown min_role → unrestricted.
fn role_meets_min(role: OrgRole, min_role: Option<&str>) -> bool {
    let Some(min) = min_role.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    let required = match min.to_ascii_lowercase().as_str() {
        "admin" => OrgRole::Admin,
        "officer" => OrgRole::Officer,
        "member" => OrgRole::Member,
        "recruit" => OrgRole::Recruit,
        _ => return true,
    };
    role.is_at_least(required)
}

fn err_forbidden_role() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: "Insufficient role for this board".into(),
        }),
    )
}

fn err_login_required() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            error: "Login required for this board".into(),
        }),
    )
}

/// Enforce min_role for a board. Anonymous only allowed when min_role is unset.
/// Applies to both read and write (judgment default: lock both).
fn enforce_board_access(
    board: &ForumBoard,
    caller_role: Option<OrgRole>,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let min = board.min_role.as_deref();
    let restricted = min.map(str::trim).filter(|s| !s.is_empty()).is_some();
    if !restricted {
        return Ok(());
    }
    match caller_role {
        None => Err(err_login_required()),
        Some(role) if role_meets_min(role, min) => Ok(()),
        Some(_) => Err(err_forbidden_role()),
    }
}

// ─── Request/Response types ─────────────────────────────────

#[derive(Deserialize)]
pub struct ListThreadsQuery {
    /// Preferred: board slug
    pub board: Option<String>,
    /// Deprecated: legacy string category
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board: Option<ForumBoard>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board: Option<ForumBoard>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<ForumCategory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_board: Option<ForumBoard>,
}

#[derive(Deserialize)]
pub struct CreateThreadRequest {
    pub title: String,
    pub content: String,
    /// Preferred: board id or slug
    pub board_id: Option<String>,
    pub board: Option<String>,
    /// Deprecated fallback
    pub category: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateCategoryRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    #[serde(default)]
    pub sort_order: i32,
}

#[derive(Deserialize)]
pub struct UpdateCategoryRequest {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub sort_order: Option<i32>,
    pub is_active: Option<bool>,
}

#[derive(Deserialize)]
pub struct CreateBoardRequest {
    pub category_id: String,
    pub parent_board_id: Option<String>,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    #[serde(default)]
    pub sort_order: i32,
}

#[derive(Deserialize)]
pub struct UpdateBoardRequest {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub sort_order: Option<i32>,
    pub is_locked: Option<bool>,
    pub is_active: Option<bool>,
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

fn err_500(e: impl ToString) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %e.to_string(), "forum internal error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "Internal server error".into(),
        }),
    )
}

fn err_400(msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse { error: msg.into() }),
    )
}

fn err_404(msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse { error: msg.into() }),
    )
}

/// GET /api/forum/tree — category → boards → sub-boards
/// Restricted boards (min_role set) are hidden unless the caller meets the role.
pub async fn forum_tree(
    State(state): State<AppState>,
    OptionalOrgMember(caller): OptionalOrgMember,
) -> Result<Json<Vec<ForumCategoryNode>>, (StatusCode, Json<ErrorResponse>)> {
    let mut tree = state.db.list_forum_tree().await.map_err(err_500)?;
    let role = caller.as_ref().map(|m| m.org_role);
    for cat in &mut tree {
        cat.boards
            .retain(|node| enforce_board_access(&node.board, role).is_ok());
        for node in &mut cat.boards {
            node.sub_boards
                .retain(|sub| enforce_board_access(sub, role).is_ok());
        }
    }
    // Drop empty categories after filtering
    tree.retain(|c| !c.boards.is_empty());
    Ok(Json(tree))
}

/// GET /api/forum/boards/:slug — board meta
pub async fn get_board(
    State(state): State<AppState>,
    OptionalOrgMember(caller): OptionalOrgMember,
    Path(slug): Path<String>,
) -> Result<Json<ForumBoard>, (StatusCode, Json<ErrorResponse>)> {
    let board = state
        .db
        .get_forum_board_by_slug(&slug)
        .await
        .map_err(err_500)?
        .ok_or_else(|| err_404("Board not found"))?;
    enforce_board_access(&board, caller.as_ref().map(|m| m.org_role))?;
    Ok(Json(board))
}

/// POST /api/forum/categories — officer+
pub async fn create_category(
    State(state): State<AppState>,
    officer: OfficerUser,
    Json(body): Json<CreateCategoryRequest>,
) -> Result<(StatusCode, Json<ForumCategory>), (StatusCode, Json<ErrorResponse>)> {
    if body.name.trim().is_empty() || body.slug.trim().is_empty() {
        return Err(err_400("name and slug required"));
    }
    let cat = state
        .db
        .create_forum_category(
            body.name.trim(),
            body.slug.trim(),
            body.description.as_deref(),
            body.sort_order,
        )
        .await
        .map_err(err_500)?;
    let _ = officer;
    Ok((StatusCode::CREATED, Json(cat)))
}

/// PATCH /api/forum/categories/:id — officer+
pub async fn update_category(
    State(state): State<AppState>,
    _officer: OfficerUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateCategoryRequest>,
) -> Result<Json<ForumCategory>, (StatusCode, Json<ErrorResponse>)> {
    let desc = body.description.as_ref().map(|o| o.as_deref());
    state
        .db
        .update_forum_category(
            &id,
            body.name.as_deref(),
            desc,
            body.sort_order,
            body.is_active,
        )
        .await
        .map(Json)
        .map_err(err_500)
}

/// POST /api/forum/boards — officer+
pub async fn create_board(
    State(state): State<AppState>,
    _officer: OfficerUser,
    Json(body): Json<CreateBoardRequest>,
) -> Result<(StatusCode, Json<ForumBoard>), (StatusCode, Json<ErrorResponse>)> {
    if body.name.trim().is_empty() || body.slug.trim().is_empty() {
        return Err(err_400("name and slug required"));
    }
    let mut category_id = body.category_id.clone();
    if let Some(pid) = body.parent_board_id.as_deref()
        && let Ok(Some(parent)) = state.db.get_forum_board(pid).await
    {
        category_id = parent.category_id;
    }
    let board = state
        .db
        .create_forum_board(
            &category_id,
            body.parent_board_id.as_deref(),
            body.name.trim(),
            body.slug.trim(),
            body.description.as_deref(),
            body.sort_order,
        )
        .await
        .map_err(err_500)?;
    Ok((StatusCode::CREATED, Json(board)))
}

/// PATCH /api/forum/boards/:id — officer+
pub async fn update_board(
    State(state): State<AppState>,
    _officer: OfficerUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateBoardRequest>,
) -> Result<Json<ForumBoard>, (StatusCode, Json<ErrorResponse>)> {
    let desc = body.description.as_ref().map(|o| o.as_deref());
    state
        .db
        .update_forum_board(
            &id,
            body.name.as_deref(),
            desc,
            body.sort_order,
            body.is_locked,
            body.is_active,
        )
        .await
        .map(Json)
        .map_err(err_500)
}

/// GET /api/forum/threads -- list threads (public unless board min_role)
pub async fn list_threads(
    State(state): State<AppState>,
    OptionalOrgMember(caller): OptionalOrgMember,
    Query(query): Query<ListThreadsQuery>,
) -> Result<Json<ThreadListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let role = caller.as_ref().map(|m| m.org_role);
    let mut board_meta = None;
    let board_id = if let Some(slug) = query.board.as_deref() {
        let b = state
            .db
            .get_forum_board_by_slug(slug)
            .await
            .map_err(err_500)?
            .ok_or_else(|| err_404("Board not found"))?;
        enforce_board_access(&b, role)?;
        let id = b.id.clone();
        board_meta = Some(b);
        Some(id)
    } else {
        None
    };

    let threads = state
        .db
        .list_forum_threads(
            board_id.as_deref(),
            query.category.as_deref(),
            query.limit.min(100),
            query.offset,
        )
        .await
        .map_err(err_500)?;

    let mut items = Vec::with_capacity(threads.len());
    for thread in threads {
        let reply_count = state.db.count_forum_replies(&thread.id).await.unwrap_or(0);
        items.push(ThreadWithReplyCount {
            thread,
            reply_count,
        });
    }

    let total = items.len();
    Ok(Json(ThreadListResponse {
        threads: items,
        total,
        board: board_meta,
    }))
}

/// GET /api/forum/threads/:id -- get thread + replies
pub async fn get_thread(
    State(state): State<AppState>,
    OptionalOrgMember(caller): OptionalOrgMember,
    Path(id): Path<String>,
    Query(query): Query<ListRepliesQuery>,
) -> Result<Json<ThreadDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let thread = state.db.get_forum_thread(&id).await.map_err(|_e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    let mut board = None;
    let mut parent_board = None;
    let mut category = None;
    if let Some(bid) = thread.board_id.as_deref()
        && let Ok(Some(b)) = state.db.get_forum_board(bid).await
    {
        enforce_board_access(&b, caller.as_ref().map(|m| m.org_role))?;
        if let Some(pid) = b.parent_board_id.as_deref() {
            parent_board = state.db.get_forum_board(pid).await.ok().flatten();
        }
        category = state
            .db
            .get_forum_category(&b.category_id)
            .await
            .ok()
            .flatten();
        board = Some(b);
    }

    let replies = state
        .db
        .list_forum_replies(&id, query.limit.min(100), query.offset)
        .await
        .map_err(err_500)?;

    let reply_count = state.db.count_forum_replies(&id).await.unwrap_or(0);

    Ok(Json(ThreadDetailResponse {
        thread,
        replies,
        reply_count,
        board,
        category,
        parent_board,
    }))
}

/// POST /api/forum/threads -- create thread (member auth)
pub async fn create_thread(
    State(state): State<AppState>,
    member: OrgMember,
    Json(body): Json<CreateThreadRequest>,
) -> Result<(StatusCode, Json<ForumThread>), (StatusCode, Json<ErrorResponse>)> {
    if body.title.trim().is_empty() || body.content.trim().is_empty() {
        return Err(err_400("Title and content are required"));
    }

    // Resolve board_id from board_id, board slug, or legacy category slug
    let board = if let Some(id) = body.board_id.as_deref().filter(|s| !s.is_empty()) {
        if let Ok(Some(b)) = state.db.get_forum_board(id).await {
            b
        } else if let Ok(Some(b)) = state.db.get_forum_board_by_slug(id).await {
            b
        } else {
            return Err(err_404("Board not found"));
        }
    } else if let Some(slug) = body.board.as_deref().filter(|s| !s.is_empty()) {
        state
            .db
            .get_forum_board_by_slug(slug)
            .await
            .map_err(err_500)?
            .ok_or_else(|| err_404("Board not found"))?
    } else if let Some(cat) = body.category.as_deref() {
        let slug = match cat {
            "general" => "general",
            "game" => "overwatch",
            "strategy" => "ow-strategy",
            "offtopic" => "offtopic-general",
            other => other,
        };
        state
            .db
            .get_forum_board_by_slug(slug)
            .await
            .map_err(err_500)?
            .ok_or_else(|| err_404("Board not found for category"))?
    } else {
        return Err(err_400("board or board_id is required"));
    };

    enforce_board_access(&board, Some(member.member.org_role))?;
    let board_id = board.id.clone();

    let thread = state
        .db
        .create_forum_thread(&body.title, &board_id, &member.member.id, &body.content)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("locked") {
                (StatusCode::FORBIDDEN, Json(ErrorResponse { error: msg }))
            } else {
                err_500(e)
            }
        })?;

    let tag = thread.category.clone();
    maybe_publish_thread_to_relay(&state, &member.member, &thread, &tag).await;

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
    let thread = state.db.get_forum_thread(&id).await.map_err(|_e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Internal error".into(),
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

    if let Some(bid) = thread.board_id.as_deref()
        && let Ok(Some(b)) = state.db.get_forum_board(bid).await
    {
        enforce_board_access(&b, Some(member.member.org_role))?;
    }

    let reply = state
        .db
        .create_forum_reply(&id, &member.member.id, &body.content)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;

    maybe_publish_reply_to_relay(
        &state,
        &member.member,
        &reply,
        thread.nostr_event_id.as_deref(),
    )
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
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
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
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
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

fn parse_extra_relay_urls(raw: &str) -> Vec<String> {
    raw.lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && (l.starts_with("ws://") || l.starts_with("wss://")))
        .collect()
}

async fn get_extra_relay_urls(state: &AppState) -> Vec<String> {
    state
        .db
        .get_settings()
        .await
        .map(|s| parse_extra_relay_urls(&s.extra_relay_urls))
        .unwrap_or_default()
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
    let mut secret_hex = match state.db.get_nostr_secret_key(&member.id).await {
        Ok(Some(key)) => key,
        _ => return,
    };
    let keys = match EventBuilder::keys_from_hex(&secret_hex) {
        Ok(k) => k,
        Err(_) => return,
    };
    secret_hex.zeroize();

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
    let extra_urls = get_extra_relay_urls(state).await;
    tokio::spawn(async move {
        if let Err(e) = publish_event_oneshot(&relay_url, relay_event.clone()).await {
            tracing::error!("Failed to publish forum thread {thread_id} to relay: {e}");
        } else {
            tracing::info!("Dual-published forum thread {thread_id} to Nostr relay");
            if let Err(e) = db
                .update_thread_nostr_event_id(&thread_id, &nostr_event_id)
                .await
            {
                tracing::error!("Failed to store nostr_event_id for thread {thread_id}: {e}");
            }
        }
        for url in extra_urls {
            if let Err(e) = publish_event_oneshot(&url, relay_event.clone()).await {
                tracing::warn!("Failed to publish thread {thread_id} to extra relay {url}: {e}");
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
    let mut secret_hex = match state.db.get_nostr_secret_key(&member.id).await {
        Ok(Some(key)) => key,
        _ => return,
    };
    let keys = match EventBuilder::keys_from_hex(&secret_hex) {
        Ok(k) => k,
        Err(_) => return,
    };
    secret_hex.zeroize();

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
    let extra_urls = get_extra_relay_urls(state).await;
    tokio::spawn(async move {
        if let Err(e) = publish_event_oneshot(&relay_url, relay_event.clone()).await {
            tracing::error!("Failed to publish forum reply {reply_id} to relay: {e}");
        } else {
            tracing::info!("Dual-published forum reply {reply_id} to Nostr relay");
        }
        for url in extra_urls {
            if let Err(e) = publish_event_oneshot(&url, relay_event.clone()).await {
                tracing::warn!("Failed to publish reply {reply_id} to extra relay {url}: {e}");
            }
        }
    });
}
