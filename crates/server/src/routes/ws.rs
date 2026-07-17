use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};
use axum_extra::extract::cookie::CookieJar;
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::collab::{JoinError, RoomManager};
use scuffed_auth::server::HasAuth;
use scuffed_site_server::state::AppState;
use scuffed_types::strategy::{
    ClientMessage, CollabUserInfo, ServerMessage, StrategyId, WsRequest, WsResponse,
};

/// Maximum WebSocket frame/message sizes
const MAX_WS_FRAME_SIZE: usize = 64 * 1024; // 64KB
const MAX_WS_MESSAGE_SIZE: usize = 256 * 1024; // 256KB

/// Extended state that includes both the original AppState and the RoomManager
#[derive(Clone)]
pub struct WsState {
    pub app: AppState,
    pub rooms: Arc<RoomManager>,
}

/// WebSocket upgrade handler
pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<WsState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !ws_origin_allowed(&state, &headers) {
        tracing::warn!("strategy WS rejected: Origin not allowed");
        return StatusCode::FORBIDDEN.into_response();
    }

    // Hard cap before upgrade so we do not accept sockets we cannot place in a room.
    if state.rooms.global_connection_count() >= crate::collab::room::MAX_GLOBAL_CONNECTIONS {
        tracing::warn!("strategy WS rejected: global connection limit");
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }

    // Try to get user from session cookie
    let user = get_user_from_cookie(&state.app, &jar).await;

    ws.max_frame_size(MAX_WS_FRAME_SIZE)
        .max_message_size(MAX_WS_MESSAGE_SIZE)
        .on_upgrade(move |socket| handle_socket(socket, state, user))
        .into_response()
}

/// Browser WS requests include Origin; must match ALLOWED_ORIGINS.
/// Missing Origin is allowed only outside PRODUCTION (native / test clients).
fn ws_origin_allowed(state: &WsState, headers: &HeaderMap) -> bool {
    let allowed = &state.app.oauth_config.allowed_origins;
    match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        Some(origin) => allowed.iter().any(|o| o == origin),
        None => !is_production(),
    }
}

fn is_production() -> bool {
    matches!(
        std::env::var("PRODUCTION").ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

/// Extract user info from session cookie
async fn get_user_from_cookie(app: &AppState, jar: &CookieJar) -> Option<CollabUserInfo> {
    let config = app.session_config();
    let token = jar.get(&config.cookie_name)?.value().to_string();

    match app.get_session_user(&token).await {
        Ok(Some(user)) => Some(CollabUserInfo {
            id: user.id,
            username: user.username,
            avatar_url: user.avatar_url,
        }),
        _ => None,
    }
}

/// Drop idle strategy connections after this many seconds without a message.
const WS_IDLE_TIMEOUT_SECS: u64 = 120;

/// Handle WebSocket connection
async fn handle_socket(socket: WebSocket, state: WsState, user: Option<CollabUserInfo>) {
    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Unique per socket so multi-tab does not clobber the same user id slot
    let connection_id = uuid::Uuid::new_v4().to_string();

    // Create channel for sending messages to this client
    let (tx, mut rx) = mpsc::channel::<WsResponse>(32);

    // Spawn task to forward messages to WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match serde_json::to_string(&msg) {
                Ok(json) => {
                    if ws_sender.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to serialize WebSocket message: {}", e);
                    if let Ok(error_json) =
                        serde_json::to_string(&WsResponse::from(ServerMessage::Error {
                            message: "Internal serialization error".into(),
                        }))
                    {
                        let _ = ws_sender.send(Message::Text(error_json.into())).await;
                    }
                }
            }
        }
    });

    // Track current room
    let mut current_room: Option<StrategyId> = None;
    let idle = tokio::time::Duration::from_secs(WS_IDLE_TIMEOUT_SECS);

    // Handle incoming messages with idle timeout (drop dead peers)
    loop {
        let msg = tokio::time::timeout(idle, ws_receiver.next()).await;
        match msg {
            Ok(Some(Ok(msg))) => match msg {
                Message::Text(text) => {
                    if let Ok(request) = serde_json::from_str::<WsRequest>(&text) {
                        let response = handle_message(
                            &state,
                            request.message,
                            &user,
                            &connection_id,
                            &mut current_room,
                            tx.clone(),
                        )
                        .await;

                        if let Some(msg) = response {
                            let response =
                                WsResponse::from(msg).with_request_id(request.request_id);
                            let _ = tx.send(response).await;
                        }
                    }
                }
                Message::Ping(_data) => {
                    let _ = tx.send(WsResponse::from(ServerMessage::Pong)).await;
                }
                Message::Close(_) => break,
                _ => {}
            },
            Ok(Some(Err(_))) | Ok(None) => break,
            Err(_) => {
                tracing::debug!(
                    connection_id = %connection_id,
                    "strategy WS idle timeout ({WS_IDLE_TIMEOUT_SECS}s)"
                );
                break;
            }
        }
    }

    // Leave room on disconnect (connection-scoped)
    if let Some(room_id) = current_room {
        state.rooms.leave_room(&room_id, &connection_id);
    }

    send_task.abort();
}

// =============================================================================
// Message dispatch
// =============================================================================

fn busy_error() -> ServerMessage {
    ServerMessage::Error {
        message: "Server busy, retry shortly".into(),
    }
}

/// Handle a single client message
async fn handle_message(
    state: &WsState,
    message: ClientMessage,
    user: &Option<CollabUserInfo>,
    connection_id: &str,
    current_room: &mut Option<StrategyId>,
    tx: mpsc::Sender<WsResponse>,
) -> Option<ServerMessage> {
    match message {
        ClientMessage::JoinRoom { strategy_id } => {
            // Check access via DB
            let user_id_ref = user.as_ref().map(|u| u.id.as_str());
            let can_access = state
                .app
                .db
                .can_access_strategy(&strategy_id, user_id_ref)
                .await
                .unwrap_or(false);

            if !can_access {
                return Some(ServerMessage::Error {
                    message: "Access denied".into(),
                });
            }

            // Get strategy from DB
            let strategy = match state.app.db.get_strategy(&strategy_id).await {
                Ok(Some(s)) => s,
                Ok(None) => {
                    return Some(ServerMessage::Error {
                        message: "Strategy not found".into(),
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to load strategy {strategy_id}: {e}");
                    return Some(ServerMessage::Error {
                        message: "Failed to load strategy".into(),
                    });
                }
            };

            // Leave current room if any (this connection only)
            if let Some(old_room) = current_room.take() {
                state.rooms.leave_room(&old_room, connection_id);
            }

            // Join new room (bounded)
            if let Some(u) = user.as_ref() {
                if let Err(err) = state.rooms.join_room(
                    &strategy_id,
                    connection_id.to_string(),
                    u.clone(),
                    tx.clone(),
                ) {
                    let message = match err {
                        JoinError::GlobalLimit => "Too many active strategy sessions".into(),
                        JoinError::RoomLimit => "Room is full".into(),
                    };
                    return Some(ServerMessage::Error { message });
                }
            }

            *current_room = Some(strategy_id.clone());

            // Get users in room
            let users = state.rooms.get_room_users(&strategy_id).unwrap_or_default();

            Some(ServerMessage::RoomJoined { strategy, users })
        }

        ClientMessage::LeaveRoom => {
            if let Some(room_id) = current_room.take() {
                state.rooms.leave_room(&room_id, connection_id);
            }
            None
        }

        ClientMessage::ElementAdd { element } => {
            let Some(room_id) = current_room.as_ref() else {
                return Some(ServerMessage::Error {
                    message: "Not in a room".into(),
                });
            };

            let Some(u) = user.as_ref() else {
                return Some(ServerMessage::Error {
                    message: "Authentication required".into(),
                });
            };

            if !state
                .app
                .db
                .can_edit_strategy(room_id, &u.id)
                .await
                .unwrap_or(false)
            {
                return Some(ServerMessage::Error {
                    message: "Permission denied".into(),
                });
            }

            let db = state.app.db.clone();
            let rid = room_id.clone();
            let elem = element.clone();
            if !state.rooms.try_spawn_persist(room_id, move || async move {
                if let Err(e) = db.add_strategy_element(&rid, &elem).await {
                    tracing::error!("Failed to persist element add for strategy {rid}: {e}");
                }
            }) {
                return Some(busy_error());
            }

            state.rooms.broadcast(
                room_id,
                connection_id,
                ServerMessage::ElementAdded {
                    by: u.id.clone(),
                    element,
                },
            );
            None
        }

        ClientMessage::ElementUpdate { id, changes } => {
            let Some(room_id) = current_room.as_ref() else {
                return Some(ServerMessage::Error {
                    message: "Not in a room".into(),
                });
            };

            let Some(u) = user.as_ref() else {
                return Some(ServerMessage::Error {
                    message: "Authentication required".into(),
                });
            };

            if !state
                .app
                .db
                .can_edit_strategy(room_id, &u.id)
                .await
                .unwrap_or(false)
            {
                return Some(ServerMessage::Error {
                    message: "Permission denied".into(),
                });
            }

            // Load → patch → save under per-strategy lock (via try_spawn_persist)
            let db = state.app.db.clone();
            let rid = room_id.clone();
            let patch = changes.clone();
            if !state.rooms.try_spawn_persist(room_id, move || async move {
                if let Ok(Some(strategy)) = db.get_strategy(&rid).await
                    && let Some(mut elem) = strategy.elements.into_iter().find(|e| e.id == id)
                {
                    elem.apply_patch(&patch);
                    if let Err(e) = db.update_strategy_element(&rid, id, &elem).await {
                        tracing::error!("Failed to persist element update for strategy {rid}: {e}");
                    }
                }
            }) {
                return Some(busy_error());
            }

            state.rooms.broadcast(
                room_id,
                connection_id,
                ServerMessage::ElementUpdated {
                    by: u.id.clone(),
                    id,
                    changes,
                },
            );
            None
        }

        ClientMessage::ElementDelete { id } => {
            let Some(room_id) = current_room.as_ref() else {
                return Some(ServerMessage::Error {
                    message: "Not in a room".into(),
                });
            };

            let Some(u) = user.as_ref() else {
                return Some(ServerMessage::Error {
                    message: "Authentication required".into(),
                });
            };

            if !state
                .app
                .db
                .can_edit_strategy(room_id, &u.id)
                .await
                .unwrap_or(false)
            {
                return Some(ServerMessage::Error {
                    message: "Permission denied".into(),
                });
            }

            let db = state.app.db.clone();
            let rid = room_id.clone();
            if !state.rooms.try_spawn_persist(room_id, move || async move {
                if let Err(e) = db.delete_strategy_element(&rid, id).await {
                    tracing::error!("Failed to persist element delete for strategy {rid}: {e}");
                }
            }) {
                return Some(busy_error());
            }

            state.rooms.broadcast(
                room_id,
                connection_id,
                ServerMessage::ElementDeleted {
                    by: u.id.clone(),
                    id,
                },
            );
            None
        }

        ClientMessage::PhaseAdd { phase } => {
            let Some(room_id) = current_room.as_ref() else {
                return Some(ServerMessage::Error {
                    message: "Not in a room".into(),
                });
            };

            let Some(u) = user.as_ref() else {
                return Some(ServerMessage::Error {
                    message: "Authentication required".into(),
                });
            };

            if !state
                .app
                .db
                .can_edit_strategy(room_id, &u.id)
                .await
                .unwrap_or(false)
            {
                return Some(ServerMessage::Error {
                    message: "Permission denied".into(),
                });
            }

            let db = state.app.db.clone();
            let rid = room_id.clone();
            let p = phase.clone();
            if !state.rooms.try_spawn_persist(room_id, move || async move {
                if let Err(e) = db.add_strategy_phase(&rid, &p).await {
                    tracing::error!("Failed to persist phase add for strategy {rid}: {e}");
                }
            }) {
                return Some(busy_error());
            }

            state.rooms.broadcast(
                room_id,
                connection_id,
                ServerMessage::PhaseAdded {
                    by: u.id.clone(),
                    phase,
                },
            );
            None
        }

        ClientMessage::PhaseUpdate { id, changes } => {
            let Some(room_id) = current_room.as_ref() else {
                return Some(ServerMessage::Error {
                    message: "Not in a room".into(),
                });
            };

            let Some(u) = user.as_ref() else {
                return Some(ServerMessage::Error {
                    message: "Authentication required".into(),
                });
            };

            if !state
                .app
                .db
                .can_edit_strategy(room_id, &u.id)
                .await
                .unwrap_or(false)
            {
                return Some(ServerMessage::Error {
                    message: "Permission denied".into(),
                });
            }

            let db = state.app.db.clone();
            let rid = room_id.clone();
            let patch = changes.clone();
            if !state.rooms.try_spawn_persist(room_id, move || async move {
                if let Ok(Some(strategy)) = db.get_strategy(&rid).await
                    && let Some(mut phase) = strategy.phases.into_iter().find(|p| p.id == id)
                {
                    phase.apply_patch(&patch);
                    if let Err(e) = db.update_strategy_phase(&rid, id, &phase).await {
                        tracing::error!("Failed to persist phase update for strategy {rid}: {e}");
                    }
                }
            }) {
                return Some(busy_error());
            }

            state.rooms.broadcast(
                room_id,
                connection_id,
                ServerMessage::PhaseUpdated {
                    by: u.id.clone(),
                    id,
                    changes,
                },
            );
            None
        }

        ClientMessage::PhaseDelete { id } => {
            let Some(room_id) = current_room.as_ref() else {
                return Some(ServerMessage::Error {
                    message: "Not in a room".into(),
                });
            };

            let Some(u) = user.as_ref() else {
                return Some(ServerMessage::Error {
                    message: "Authentication required".into(),
                });
            };

            if !state
                .app
                .db
                .can_edit_strategy(room_id, &u.id)
                .await
                .unwrap_or(false)
            {
                return Some(ServerMessage::Error {
                    message: "Permission denied".into(),
                });
            }

            let db = state.app.db.clone();
            let rid = room_id.clone();
            if !state.rooms.try_spawn_persist(room_id, move || async move {
                if let Err(e) = db.delete_strategy_phase(&rid, id).await {
                    tracing::error!("Failed to persist phase delete for strategy {rid}: {e}");
                }
            }) {
                return Some(busy_error());
            }

            state.rooms.broadcast(
                room_id,
                connection_id,
                ServerMessage::PhaseDeleted {
                    by: u.id.clone(),
                    id,
                },
            );
            None
        }

        ClientMessage::CursorMove { position } => {
            if let (Some(room_id), Some(u)) = (current_room.as_ref(), user.as_ref()) {
                state.rooms.broadcast(
                    room_id,
                    connection_id,
                    ServerMessage::CursorMoved {
                        user_id: u.id.clone(),
                        position,
                    },
                );
            }
            None
        }

        ClientMessage::Ping => Some(ServerMessage::Pong),
    }
}
