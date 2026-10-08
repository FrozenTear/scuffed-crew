use axum::{
    extract::{
        ConnectInfo, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;
use futures::{SinkExt, StreamExt};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

use dashmap::DashMap;
use scuffed_site_server::rate_limit::TrustedProxyIpKeyExtractor;

use crate::collab::{JoinError, RoomManager};
use scuffed_auth::server::HasAuth;
use scuffed_site_server::state::AppState;
use scuffed_types::strategy::{
    ClientMessage, CollabUserInfo, ServerMessage, StrategyId, WsRequest, WsResponse,
};

/// Maximum WebSocket frame/message sizes
pub(crate) const MAX_WS_FRAME_SIZE: usize = 64 * 1024; // 64KB
pub(crate) const MAX_WS_MESSAGE_SIZE: usize = 256 * 1024; // 256KB

/// Strategy sockets that never send JoinRoom are closed after this long.
pub const WS_JOIN_TIMEOUT: Duration = Duration::from_secs(10);

/// Default global cap for strategy sockets (`WS_MAX_CONNECTIONS`).
pub const DEFAULT_WS_MAX_CONNECTIONS: usize = 512;
/// Default concurrent sockets per client IP (`WS_MAX_PER_IP`).
pub const DEFAULT_WS_MAX_PER_IP: usize = 32;

/// Extended state that includes both the original AppState and the RoomManager
#[derive(Clone)]
pub struct WsState {
    pub app: AppState,
    pub rooms: Arc<RoomManager>,
    pub admission: Arc<WsAdmission>,
    /// Join deadline for `/api/strategy/ws`.
    pub join_timeout: Duration,
}

/// Global semaphore plus a per-client-IP semaphore. One permit of each is held
/// for the whole life of a strategy socket.
#[derive(Clone)]
pub struct WsAdmission {
    global: Arc<Semaphore>,
    per_ip_limit: usize,
    buckets: Arc<DashMap<IpAddr, Arc<Semaphore>>>,
    extractor: TrustedProxyIpKeyExtractor,
}

pub struct WsPermit {
    _global: OwnedSemaphorePermit,
    _ip: OwnedSemaphorePermit,
}

#[derive(Debug)]
pub(crate) enum WsAdmitError {
    Global,
    PerIp,
}

impl WsAdmission {
    pub fn new(global: usize, per_ip: usize, extractor: TrustedProxyIpKeyExtractor) -> Self {
        Self {
            global: Arc::new(Semaphore::new(global.max(1))),
            per_ip_limit: per_ip.max(1),
            buckets: Arc::new(DashMap::new()),
            extractor,
        }
    }

    /// Read `WS_MAX_CONNECTIONS` (default 512) and `WS_MAX_PER_IP` (default 32).
    pub fn limits_from_env() -> (usize, usize) {
        (
            env_usize("WS_MAX_CONNECTIONS", DEFAULT_WS_MAX_CONNECTIONS),
            env_usize("WS_MAX_PER_IP", DEFAULT_WS_MAX_PER_IP),
        )
    }

    /// `peer` is the TCP socket. Forwarded headers count only when that peer
    /// is a trusted proxy (`TRUSTED_PROXIES` / loopback), same as the HTTP
    /// rate limiter.
    pub fn try_acquire(&self, peer: IpAddr, headers: &HeaderMap) -> Result<WsPermit, WsAdmitError> {
        let ip = self.extractor.client_ip(peer, headers);
        let global = self
            .global
            .clone()
            .try_acquire_owned()
            .map_err(|_| WsAdmitError::Global)?;
        let bucket = self
            .buckets
            .entry(ip)
            .or_insert_with(|| Arc::new(Semaphore::new(self.per_ip_limit)))
            .clone();
        match bucket.try_acquire_owned() {
            Ok(ip_permit) => Ok(WsPermit {
                _global: global,
                _ip: ip_permit,
            }),
            Err(_) => Err(WsAdmitError::PerIp),
        }
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    match std::env::var(name) {
        Ok(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return default;
            }
            match trimmed.parse::<usize>() {
                Ok(0) => {
                    tracing::warn!("{name}=0 is not a connection cap; using {default}");
                    default
                }
                Ok(n) => n,
                Err(_) => {
                    tracing::warn!("{name}={raw:?} is not a number; using {default}");
                    default
                }
            }
        }
        Err(_) => default,
    }
}

pub(crate) fn ws_over_cap(err: WsAdmitError) -> Response {
    let status = match err {
        WsAdmitError::Global => StatusCode::SERVICE_UNAVAILABLE,
        WsAdmitError::PerIp => StatusCode::TOO_MANY_REQUESTS,
    };
    (
        status,
        [(header::RETRY_AFTER, HeaderValue::from_static("1"))],
        "too many websocket connections",
    )
        .into_response()
}

/// WebSocket upgrade handler for strategy collab.
///
/// Every accepted socket holds a global and per-IP permit until the handler
/// returns, including sockets that never join a room. A socket that has not
/// sent JoinRoom within [`WS_JOIN_TIMEOUT`] is closed.
pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<WsState>,
    jar: CookieJar,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    // Same fail-closed gate as strategy REST. Patch notes are not on this path.
    if let Err(status) = super::strategy::ensure_strategies_enabled(&state.app).await {
        return status.into_response();
    }

    if !ws_origin_allowed(&state, &headers) {
        tracing::warn!("strategy WS rejected: Origin not allowed");
        return StatusCode::FORBIDDEN.into_response();
    }

    let permit = match state.admission.try_acquire(peer.ip(), &headers) {
        Ok(permit) => permit,
        Err(err) => {
            tracing::warn!("strategy WS rejected: connection cap");
            return ws_over_cap(err);
        }
    };

    // Try to get user from session cookie
    let user = get_user_from_cookie(&state.app, &jar).await;

    ws.max_frame_size(MAX_WS_FRAME_SIZE)
        .max_message_size(MAX_WS_MESSAGE_SIZE)
        .on_upgrade(move |socket| handle_socket(socket, state, user, permit))
        .into_response()
}

/// Browser WS requests include Origin; must match ALLOWED_ORIGINS.
/// Missing Origin is allowed only outside PRODUCTION (native / test clients).
///
/// `OAuthConfig::from_env` treats blank `ALLOWED_ORIGINS` as unset (F-API-004)
/// so this list is never `[""]` from compose's empty default.
pub(crate) fn ws_origin_allowed(state: &WsState, headers: &HeaderMap) -> bool {
    origin_is_allowed(
        &state.app.oauth_config.allowed_origins,
        headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()),
        scuffed_auth::is_production_env(),
    )
}

fn origin_is_allowed(allowed: &[String], origin: Option<&str>, production: bool) -> bool {
    match origin {
        Some(origin) => allowed.iter().any(|o| o == origin),
        None => !production,
    }
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

/// Handle WebSocket connection. `permit` is held until this function returns.
async fn handle_socket(
    socket: WebSocket,
    state: WsState,
    user: Option<CollabUserInfo>,
    permit: WsPermit,
) {
    let _permit = permit;
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

    // Track current room. `joined_a_room` stays set after the first successful
    // JoinRoom so LeaveRoom does not re-arm the connect-time join deadline.
    let mut current_room: Option<StrategyId> = None;
    let mut joined_a_room = false;
    let join_deadline = tokio::time::Instant::now() + state.join_timeout;
    let idle = tokio::time::Duration::from_secs(WS_IDLE_TIMEOUT_SECS);

    // Handle incoming messages with idle timeout (drop dead peers).
    // Pings do not extend the join deadline: it is fixed at connect.
    loop {
        let wait = if joined_a_room {
            idle
        } else {
            join_deadline.saturating_duration_since(tokio::time::Instant::now())
        };
        if !joined_a_room && wait.is_zero() {
            tracing::info!(
                connection_id = %connection_id,
                "strategy WS closed: no JoinRoom within {:?}",
                state.join_timeout
            );
            break;
        }
        let msg = tokio::time::timeout(wait, ws_receiver.next()).await;
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
                        if current_room.is_some() {
                            joined_a_room = true;
                        }

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
            Err(_) if !joined_a_room => {
                tracing::info!(
                    connection_id = %connection_id,
                    "strategy WS closed: no JoinRoom within {:?}",
                    state.join_timeout
                );
                break;
            }
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

    // The old WS matcher only accepted 1/true/TRUE/yes/YES. `on`, `True`,
    // and a padded `yes` are production under `scuffed_auth::is_production_env`
    // and must require Origin.
    #[test]
    fn missing_origin_rejected_for_shared_production_variants() {
        let allowed = vec!["https://ow.scuffedcrew.no".to_string()];
        for v in [
            "1",
            "true",
            "TRUE",
            "True",
            "yes",
            "YES",
            "on",
            "ON",
            " yes ",
            "production",
        ] {
            assert!(
                scuffed_auth::production_value_enabled(v),
                "{v:?} must count as production"
            );
            assert!(
                !origin_is_allowed(&allowed, None, scuffed_auth::production_value_enabled(v)),
                "PRODUCTION={v:?} must reject a missing Origin"
            );
        }
    }

    #[test]
    fn missing_origin_allowed_for_falsy_production_values() {
        let allowed = vec!["http://localhost:3000".to_string()];
        for v in ["", " ", "0", "false", "FALSE", "False", "no", "off", "OFF"] {
            assert!(
                !scuffed_auth::production_value_enabled(v),
                "{v:?} must not count as production"
            );
            assert!(
                origin_is_allowed(&allowed, None, scuffed_auth::production_value_enabled(v)),
                "PRODUCTION={v:?} may omit Origin"
            );
        }
    }
}

#[cfg(test)]
mod strategies_gate_tests {
    use super::*;
    use axum::Router;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::get;
    use scuffed_auth::SessionConfig;
    use scuffed_db::Database;
    use scuffed_db::migrations::run_migrations;
    use scuffed_site_server::rate_limit::TrustedProxyIpKeyExtractor;
    use scuffed_site_server::state::OAuthConfig;
    use std::net::{IpAddr, SocketAddr};
    use std::path::PathBuf;
    use std::time::Duration;
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
            login_lockout: scuffed_site_server::login_lockout::LoginLockout::new(),
            crypto: None,
            relay_url: None,
            dm_events: None,
            nip05_domain: None,
            nip05_republish_enabled: false,
            public_settings: scuffed_site_server::state::PublicSettingsCache::new(),
            leaderboard_cache: scuffed_site_server::leaderboard_cache::LeaderboardCache::from_env(),
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

    fn test_ws_state(
        app: AppState,
        global: usize,
        per_ip: usize,
        join_timeout: Duration,
    ) -> WsState {
        WsState {
            app,
            rooms: Arc::new(RoomManager::with_global_limit(global)),
            admission: Arc::new(WsAdmission::new(
                global,
                per_ip,
                TrustedProxyIpKeyExtractor::with_proxies("10.89.0.1"),
            )),
            join_timeout,
        }
    }

    fn ws_router(state: AppState) -> Router {
        Router::new()
            .route("/api/strategy/ws", get(websocket_handler))
            .with_state(test_ws_state(state, 512, 32, WS_JOIN_TIMEOUT))
    }

    /// Real TCP handshake. `tower::oneshot` has no `hyper::upgrade::OnUpgrade`,
    /// so the extractor rejects with 426 before this handler runs.
    async fn handshake_status(state: AppState) -> StatusCode {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let app = ws_router(state).into_make_service_with_connect_info::<std::net::SocketAddr>();
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

    #[test]
    fn default_caps_match_the_documented_env_defaults() {
        assert_eq!(DEFAULT_WS_MAX_CONNECTIONS, 512);
        assert_eq!(DEFAULT_WS_MAX_PER_IP, 32);
        assert_eq!(WS_JOIN_TIMEOUT, Duration::from_secs(10));
    }

    #[test]
    fn forwarded_client_ips_are_separate_buckets_and_untrusted_xff_is_ignored() {
        let admission =
            WsAdmission::new(8, 1, TrustedProxyIpKeyExtractor::with_proxies("10.89.0.1"));
        let trusted: IpAddr = "10.89.0.1".parse().unwrap();
        let mut home_a = HeaderMap::new();
        home_a.insert("x-forwarded-for", "203.0.113.10".parse().unwrap());
        let mut home_b = HeaderMap::new();
        home_b.insert("x-forwarded-for", "203.0.113.20".parse().unwrap());

        let permit_a = admission
            .try_acquire(trusted, &home_a)
            .expect("first client");
        let permit_b = admission
            .try_acquire(trusted, &home_b)
            .expect("second forwarded client is a different bucket");
        assert!(
            matches!(
                admission.try_acquire(trusted, &home_a),
                Err(WsAdmitError::PerIp)
            ),
            "same forwarded client must not get a second socket"
        );
        drop(permit_a);
        drop(permit_b);

        let untrusted: IpAddr = "198.51.100.8".parse().unwrap();
        let held = admission
            .try_acquire(untrusted, &home_a)
            .expect("untrusted peer is keyed by its socket address");
        assert!(
            matches!(
                admission.try_acquire(untrusted, &home_b),
                Err(WsAdmitError::PerIp)
            ),
            "an untrusted peer's X-Forwarded-For must not open another bucket"
        );
        drop(held);

        let other: IpAddr = "198.51.100.9".parse().unwrap();
        let _peer_a = admission.try_acquire(untrusted, &home_a).unwrap();
        let _peer_b = admission
            .try_acquire(other, &home_a)
            .expect("different untrusted peers stay separate even with the same XFF");
    }

    async fn serve_ws(
        state: AppState,
        global: usize,
        per_ip: usize,
        join_timeout: Duration,
    ) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let app = Router::new()
            .route("/api/strategy/ws", get(websocket_handler))
            .with_state(test_ws_state(state, global, per_ip, join_timeout))
            .into_make_service_with_connect_info::<SocketAddr>();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        addr
    }

    /// HTTP upgrade. On 101 the socket is returned still open so the permit stays held.
    async fn ws_upgrade(
        addr: SocketAddr,
        path: &str,
        forwarded_for: Option<&str>,
    ) -> (StatusCode, Option<u64>, Option<tokio::net::TcpStream>) {
        let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let extra = forwarded_for
            .map(|ip| format!("X-Forwarded-For: {ip}\r\n"))
            .unwrap_or_default();
        let req = format!(
            "GET {path} HTTP/1.1\r\n\
             Host: {addr}\r\n\
             Origin: http://localhost:3000\r\n\
             Connection: Upgrade\r\n\
             Upgrade: websocket\r\n\
             Sec-WebSocket-Version: 13\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             {extra}\
             \r\n"
        );
        stream.write_all(req.as_bytes()).await.expect("write");
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut byte))
                .await
                .expect("header timeout")
                .expect("read");
            if n == 0 {
                break;
            }
            buf.push(byte[0]);
            if buf.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let text = String::from_utf8_lossy(&buf);
        let mut lines = text.split("\r\n");
        let code: u16 = lines
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or_else(|| panic!("no status in {text:?}"));
        let status = StatusCode::from_u16(code).unwrap();
        let mut retry_after = None;
        for line in lines {
            if line.is_empty() {
                break;
            }
            let lower = line.to_ascii_lowercase();
            if let Some(value) = lower.strip_prefix("retry-after:") {
                retry_after = value.trim().parse().ok();
            }
        }
        if status == StatusCode::SWITCHING_PROTOCOLS {
            (status, retry_after, Some(stream))
        } else {
            (status, retry_after, None)
        }
    }

    #[tokio::test]
    async fn unjoined_strategy_socket_counts_and_over_cap_sets_retry_after() {
        let state = test_state().await;
        set_strategies_enabled(&state, true).await;
        let addr = serve_ws(state, 1, 32, Duration::from_millis(200)).await;

        let (status, _, held) = ws_upgrade(addr, "/api/strategy/ws", None).await;
        assert_eq!(status, StatusCode::SWITCHING_PROTOCOLS);
        let _held = held.expect("first socket stays open");

        let (status, retry, _) = ws_upgrade(addr, "/api/strategy/ws", None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            retry.unwrap_or(0) >= 1,
            "over-cap upgrade needs Retry-After >= 1, got {retry:?}"
        );
    }

    #[tokio::test]
    async fn trusted_proxy_forwarded_ips_do_not_share_a_bucket() {
        let state = test_state().await;
        set_strategies_enabled(&state, true).await;
        // Loopback is a trusted proxy, so X-Forwarded-For is the client key.
        let addr = serve_ws(state, 8, 1, WS_JOIN_TIMEOUT).await;

        let (status, _, held_a) = ws_upgrade(addr, "/api/strategy/ws", Some("203.0.113.10")).await;
        assert_eq!(status, StatusCode::SWITCHING_PROTOCOLS);
        let _held_a = held_a.expect("first forwarded client");

        let (status, retry, _) = ws_upgrade(addr, "/api/strategy/ws", Some("203.0.113.10")).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert!(retry.unwrap_or(0) >= 1);

        let (status, _, held_b) = ws_upgrade(addr, "/api/strategy/ws", Some("203.0.113.20")).await;
        assert_eq!(
            status,
            StatusCode::SWITCHING_PROTOCOLS,
            "a different forwarded client must get its own bucket"
        );
        drop(held_b);
    }

    /// An unjoined strategy socket is closed once `join_timeout` elapses.
    /// The deadline is injected so the test does not sleep the production 10s.
    #[tokio::test]
    async fn unjoined_strategy_socket_closes_at_join_deadline() {
        let state = test_state().await;
        set_strategies_enabled(&state, true).await;
        let join = Duration::from_millis(40);
        let addr = serve_ws(state, 8, 32, join).await;

        let (status, _, held) = ws_upgrade(addr, "/api/strategy/ws", None).await;
        assert_eq!(status, StatusCode::SWITCHING_PROTOCOLS);
        let mut stream = held.expect("socket stays open until the deadline");

        let started = std::time::Instant::now();
        let mut buf = [0u8; 64];
        let closed = loop {
            match tokio::time::timeout(Duration::from_millis(30), stream.read(&mut buf)).await {
                Ok(Ok(0)) => break true,
                Ok(Ok(_)) if buf[0] & 0x0f == 0x8 => break true,
                Ok(Ok(n)) => panic!("unexpected payload before close: {n} bytes"),
                Ok(Err(err)) => panic!("read failed: {err}"),
                Err(_) if started.elapsed() > Duration::from_secs(2) => break false,
                Err(_) => continue,
            }
        };
        assert!(
            closed,
            "unjoined strategy socket was still open after the join deadline"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "close took too long for an injected deadline"
        );
    }
}
