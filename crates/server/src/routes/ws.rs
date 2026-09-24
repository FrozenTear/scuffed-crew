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
    // Same fail-closed gate as strategy REST. Patch notes are not on this path.
    if let Err(status) = super::strategy::ensure_strategies_enabled(&state.app).await {
        return status.into_response();
    }

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
///
/// `OAuthConfig::from_env` treats blank `ALLOWED_ORIGINS` as unset (F-API-004)
/// so this list is never `[""]` from compose's empty default.
fn ws_origin_allowed(state: &WsState, headers: &HeaderMap) -> bool {
    origin_is_allowed(
        &state.app.oauth_config.allowed_origins,
        headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()),
        is_production(),
    )
}

fn origin_is_allowed(allowed: &[String], origin: Option<&str>, production: bool) -> bool {
    match origin {
        Some(origin) => allowed.iter().any(|o| o == origin),
        None => !production,
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
            if let Some(u) = user.as_ref()
                && let Err(err) = state.rooms.join_room(
                    &strategy_id,
                    connection_id.to_string(),
                    u.clone(),
                    tx.clone(),
                )
            {
                let message = match err {
                    JoinError::GlobalLimit => "Too many active strategy sessions".into(),
                    JoinError::RoomLimit => "Room is full".into(),
                };
                return Some(ServerMessage::Error { message });
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

#[cfg(test)]
mod origin_tests {
    use super::origin_is_allowed;

    #[test]
    fn explicit_origin_must_match() {
        let allowed = vec!["https://ow.scuffedcrew.no".to_string()];
        assert!(origin_is_allowed(
            &allowed,
            Some("https://ow.scuffedcrew.no"),
            true
        ));
        assert!(!origin_is_allowed(
            &allowed,
            Some("http://localhost:3000"),
            true
        ));
    }

    #[test]
    fn empty_string_in_allowlist_does_not_match_browser_origin() {
        // Documents the F-API-004 landmine: compose `ALLOWED_ORIGINS=""` used
        // to parse as `[""]`, which never equals a real Origin → 403.
        let landmine = vec![String::new()];
        assert!(!origin_is_allowed(
            &landmine,
            Some("http://127.0.0.1:3000"),
            true
        ));
    }

    #[test]
    fn missing_origin_allowed_only_outside_production() {
        let allowed = vec!["http://localhost:3000".to_string()];
        assert!(origin_is_allowed(&allowed, None, false));
        assert!(!origin_is_allowed(&allowed, None, true));
    }
}

#[cfg(test)]
mod strategies_gate_tests {
    use super::*;
    use axum::Router;
    use axum::http::StatusCode;
    use axum::routing::get;
    use scuffed_auth::SessionConfig;
    use scuffed_db::Database;
    use scuffed_db::migrations::run_migrations;
    use scuffed_site_server::state::OAuthConfig;
    use std::path::PathBuf;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::super::strategy::strategies_gate_status;

    async fn test_state() -> AppState {
        let db = Database::connect_memory().await.expect("mem db");
        run_migrations(&db.client).await.expect("migrations");
        AppState {
            db: Arc::new(db),
            session_config: SessionConfig::default(),
            oauth_config: OAuthConfig {
                discord_client_id: String::new(),
                discord_client_secret: String::new(),
                google_client_id: String::new(),
                google_client_secret: String::new(),
                redirect_base_url: "http://localhost:3000".into(),
                allowed_origins: vec!["http://localhost:3000".into()],
            },
            upload_dir: PathBuf::from("/tmp/scuffed-test-uploads"),
            notifier: None,
            nostr_challenge_key: [0u8; 32],
            consumed_challenges: scuffed_site_server::challenge_store::ConsumedChallengeStore::new(
            ),
            nostr_rate_limiter: scuffed_site_server::nostr_rate_limit::NostrRateLimiter::new(),
            crypto: None,
            relay_url: None,
            dm_events: None,
            nip05_domain: None,
            nip05_republish_enabled: false,
        }
    }

    async fn set_strategies_enabled(state: &AppState, enabled: bool) {
        state
            .db
            .update_settings(
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(enabled),
                None,
            )
            .await
            .expect("update strategies_enabled");
    }

    fn ws_router(state: AppState) -> Router {
        let ws_state = WsState {
            app: state,
            rooms: Arc::new(RoomManager::new()),
        };
        Router::new()
            .route("/api/strategy/ws", get(websocket_handler))
            .with_state(ws_state)
    }

    /// Real TCP handshake. `tower::oneshot` has no `hyper::upgrade::OnUpgrade`,
    /// so the extractor rejects with 426 before this handler runs.
    async fn handshake_status(state: AppState) -> StatusCode {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let app = ws_router(state).into_make_service();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });

        let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let req = format!(
            "GET /api/strategy/ws HTTP/1.1\r\n\
             Host: {addr}\r\n\
             Origin: http://localhost:3000\r\n\
             Connection: Upgrade\r\n\
             Upgrade: websocket\r\n\
             Sec-WebSocket-Version: 13\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             \r\n"
        );
        stream.write_all(req.as_bytes()).await.expect("write");
        let mut buf = [0u8; 1024];
        let n = tokio::time::timeout(std::time::Duration::from_secs(2), stream.read(&mut buf))
            .await
            .expect("handshake timed out")
            .expect("read");
        server.abort();

        let text = String::from_utf8_lossy(&buf[..n]);
        let code: u16 = text
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or_else(|| panic!("no HTTP status in {text:?}"));
        StatusCode::from_u16(code).unwrap_or_else(|_| panic!("invalid status {code} in {text:?}"))
    }

    #[test]
    fn gate_status_matches_rest_fail_closed() {
        assert_eq!(strategies_gate_status(Ok(true)), Ok(()));
        assert_eq!(
            strategies_gate_status(Ok(false)),
            Err(StatusCode::NOT_FOUND)
        );
        assert_eq!(
            strategies_gate_status(Err(())),
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        );
    }

    #[tokio::test]
    async fn strategies_disabled_rejects_ws_upgrade() {
        let state = test_state().await;
        set_strategies_enabled(&state, false).await;
        assert_eq!(handshake_status(state).await, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn strategies_enabled_accepts_ws_upgrade() {
        let state = test_state().await;
        set_strategies_enabled(&state, true).await;
        assert_eq!(
            handshake_status(state).await,
            StatusCode::SWITCHING_PROTOCOLS
        );
    }
}
