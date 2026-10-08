//! Same-origin Nostr relay socket (`GET /relay`).
//!
//! The browser client (`crates/app/src/state/nostr.rs`) opens this and may sit
//! with zero subscriptions. It counts toward the shared global and per-IP
//! caps for its whole lifetime. There is no join timeout and no server-side
//! idle close on this route — that deadline belongs only to
//! `/api/strategy/ws`, whose client sends JoinRoom on open.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use std::net::SocketAddr;

use super::ws::{self, WsPermit, WsState};

/// WebSocket upgrade for the same-origin relay.
pub async fn relay_websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<WsState>,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    if !ws::ws_origin_allowed(&state, &headers) {
        tracing::warn!("relay WS rejected: Origin not allowed");
        return StatusCode::FORBIDDEN.into_response();
    }

    let permit = match state.admission.try_acquire(peer.ip(), &headers) {
        Ok(permit) => permit,
        Err(err) => {
            tracing::warn!("relay WS rejected: connection cap");
            return ws::ws_over_cap(err);
        }
    };

    let upstream = state.app.relay_url.clone();
    ws.max_frame_size(ws::MAX_WS_FRAME_SIZE)
        .max_message_size(ws::MAX_WS_MESSAGE_SIZE)
        .on_upgrade(move |socket| hold_relay(socket, upstream, permit))
        .into_response()
}

/// Hold `permit` until the client disconnects. No timer runs here.
async fn hold_relay(socket: WebSocket, upstream: Option<String>, permit: WsPermit) {
    let _permit = permit;
    match upstream {
        Some(url) => proxy_relay(socket, &url).await,
        None => read_until_client_closes(socket).await,
    }
}

async fn read_until_client_closes(mut socket: WebSocket) {
    while let Some(Ok(msg)) = socket.recv().await {
        if matches!(msg, Message::Close(_)) {
            break;
        }
    }
}

/// Byte pipe to `NOSTR_RELAY_URL`. Either side ending closes the other.
/// Nothing in this function applies an idle or join timeout.
async fn proxy_relay(client: WebSocket, url: &str) {
    let upstream = match tokio_tungstenite::connect_async(url).await {
        Ok((stream, _)) => stream,
        Err(e) => {
            tracing::warn!(error = %e, "nostr relay upstream unavailable");
            return;
        }
    };

    let (mut client_tx, mut client_rx) = client.split();
    let (mut up_tx, mut up_rx) = upstream.split();

    let client_to_up = async {
        while let Some(Ok(msg)) = client_rx.next().await {
            if matches!(msg, Message::Close(_)) {
                break;
            }
            let Some(out) = to_tungstenite(msg) else {
                break;
            };
            if up_tx.send(out).await.is_err() {
                break;
            }
        }
    };
    let up_to_client = async {
        while let Some(Ok(msg)) = up_rx.next().await {
            let Some(out) = to_axum(msg) else {
                break;
            };
            if client_tx.send(out).await.is_err() {
                break;
            }
        }
    };

    tokio::select! {
        _ = client_to_up => {}
        _ = up_to_client => {}
    }
}

fn to_tungstenite(msg: Message) -> Option<tokio_tungstenite::tungstenite::Message> {
    use tokio_tungstenite::tungstenite::Message as T;
    Some(match msg {
        Message::Text(text) => T::Text(text.to_string().into()),
        Message::Binary(bin) => T::Binary(bin.to_vec().into()),
        Message::Ping(payload) => T::Ping(payload.to_vec().into()),
        Message::Pong(payload) => T::Pong(payload.to_vec().into()),
        Message::Close(_) => return None,
    })
}

fn to_axum(msg: tokio_tungstenite::tungstenite::Message) -> Option<Message> {
    use tokio_tungstenite::tungstenite::Message as T;
    Some(match msg {
        T::Text(text) => Message::Text(text.to_string().into()),
        T::Binary(bin) => Message::Binary(bin.to_vec().into()),
        T::Ping(payload) => Message::Ping(payload.to_vec().into()),
        T::Pong(payload) => Message::Pong(payload.to_vec().into()),
        T::Close(_) => return None,
        T::Frame(_) => return None,
    })
}
