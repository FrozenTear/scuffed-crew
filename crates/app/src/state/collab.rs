//! WebSocket client for real-time strategy collaboration.
//!
//! Unlike the Leptos version, this module does NOT use framework signals internally.
//! Instead, `CollabManager` communicates state changes via callbacks (`on_state_change`,
//! `on_message`). The component layer wraps it in Dioxus signals using `use_hook`.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, WebSocket};

use scuffed_types::strategy::{
    ClientMessage, Position, ServerMessage, StrategyId, WsRequest, WsResponse,
};

// =============================================================================
// Constants
// =============================================================================

/// Cursor update throttle interval in milliseconds.
const CURSOR_THROTTLE_MS: u32 = 100;

/// Maximum reconnection attempts before giving up.
const MAX_RECONNECT_ATTEMPTS: u32 = 10;

/// Base delay for reconnection backoff (milliseconds).
const BASE_RECONNECT_DELAY_MS: u32 = 1000;

/// Maximum reconnection delay (milliseconds).
const MAX_RECONNECT_DELAY_MS: u32 = 30_000;

// =============================================================================
// ConnectionState
// =============================================================================

/// WebSocket connection state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting(u32), // attempt number
    Error,
}

impl ConnectionState {
    pub fn is_connected(&self) -> bool {
        matches!(self, ConnectionState::Connected)
    }

    pub fn is_reconnecting(&self) -> bool {
        matches!(self, ConnectionState::Reconnecting(_))
    }

    pub fn display_text(&self) -> &'static str {
        match self {
            ConnectionState::Disconnected => "Disconnected",
            ConnectionState::Connecting => "Connecting...",
            ConnectionState::Connected => "Connected",
            ConnectionState::Reconnecting(_) => "Reconnecting...",
            ConnectionState::Error => "Connection error",
        }
    }
}

// =============================================================================
// Reconnection helpers
// =============================================================================

/// Calculate reconnection delay with exponential backoff.
fn reconnect_delay(attempt: u32) -> u32 {
    let delay = BASE_RECONNECT_DELAY_MS.saturating_mul(2u32.saturating_pow(attempt.min(5)));
    delay.min(MAX_RECONNECT_DELAY_MS)
}

// =============================================================================
// SharedWsState
// =============================================================================

/// Shared state for the WS connection, accessible from JS closures.
///
/// This is `Rc`-wrapped so that `wasm_bindgen::Closure` callbacks can capture it.
/// No Dioxus signals — state changes are communicated via the `on_state_change`
/// and `on_message` callbacks.
struct SharedWsState {
    socket: RefCell<Option<WebSocket>>,
    reconnect_attempts: RefCell<u32>,
    current_room: RefCell<Option<StrategyId>>,
    reconnect_timer_id: RefCell<Option<i32>>,
    intentional_disconnect: RefCell<bool>,
    on_state_change: Rc<dyn Fn(ConnectionState)>,
    on_message: Rc<dyn Fn(ServerMessage)>,
}

// =============================================================================
// WebSocket setup (free function for reuse in reconnection)
// =============================================================================

/// Set up a new WebSocket connection with all handlers.
///
/// Extracted as a free function so it can be called from both `connect()` and
/// the reconnection timer callback.
fn setup_ws_connection(shared: &Rc<SharedWsState>, strategy_id: &StrategyId) {
    (shared.on_state_change)(ConnectionState::Connecting);

    let Some(window) = web_sys::window() else {
        tracing::error!("No browser window available for WebSocket connection");
        (shared.on_state_change)(ConnectionState::Disconnected);
        return;
    };
    let location = window.location();
    let protocol = if location.protocol().unwrap_or_default() == "https:" {
        "wss:"
    } else {
        "ws:"
    };
    let host = match location.host() {
        Ok(h) => h,
        Err(_) => {
            tracing::error!("Failed to read location.host for WebSocket URL");
            (shared.on_state_change)(ConnectionState::Disconnected);
            return;
        }
    };
    let url = format!("{}//{}/api/strategy/ws", protocol, host);

    match WebSocket::new(&url) {
        Ok(ws) => {
            ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

            // -- On open: join the room -----------------------------------------------
            let shared_open = shared.clone();
            let strategy_id_open = strategy_id.clone();
            let onopen = Closure::wrap(Box::new(move |_: JsValue| {
                tracing::info!("WebSocket connected");
                (shared_open.on_state_change)(ConnectionState::Connected);
                *shared_open.reconnect_attempts.borrow_mut() = 0;

                let msg = WsRequest {
                    request_id: None,
                    message: ClientMessage::JoinRoom {
                        strategy_id: strategy_id_open.clone(),
                    },
                };
                if let Some(socket) = shared_open.socket.borrow().as_ref() {
                    if let Ok(json) = serde_json::to_string(&msg) {
                        let _ = socket.send_with_str(&json);
                    }
                }
            }) as Box<dyn FnMut(JsValue)>);
            ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
            onopen.forget();

            // -- On message: deserialize and forward ----------------------------------
            let shared_msg = shared.clone();
            let onmessage = Closure::wrap(Box::new(move |e: MessageEvent| {
                if let Ok(text) = e.data().dyn_into::<js_sys::JsString>() {
                    let text: String = text.into();
                    match serde_json::from_str::<WsResponse>(&text) {
                        Ok(response) => {
                            (shared_msg.on_message)(response.message);
                        }
                        Err(err) => {
                            tracing::warn!("Failed to deserialize WS message: {}", err);
                        }
                    }
                }
            }) as Box<dyn FnMut(MessageEvent)>);
            ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
            onmessage.forget();

            // -- On error -------------------------------------------------------------
            let shared_err = shared.clone();
            let onerror = Closure::wrap(Box::new(move |_: JsValue| {
                tracing::error!("WebSocket error");
                (shared_err.on_state_change)(ConnectionState::Error);
            }) as Box<dyn FnMut(JsValue)>);
            ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
            onerror.forget();

            // -- On close: trigger reconnection with exponential backoff --------------
            let shared_close = shared.clone();
            let onclose = Closure::wrap(Box::new(move |_: JsValue| {
                tracing::info!("WebSocket closed");
                *shared_close.socket.borrow_mut() = None;

                if *shared_close.intentional_disconnect.borrow() {
                    (shared_close.on_state_change)(ConnectionState::Disconnected);
                    return;
                }

                let attempts = *shared_close.reconnect_attempts.borrow();
                let room = shared_close.current_room.borrow().clone();

                if attempts < MAX_RECONNECT_ATTEMPTS {
                    if let Some(room_id) = room {
                        let delay = reconnect_delay(attempts);
                        let new_attempt = attempts + 1;
                        (shared_close.on_state_change)(ConnectionState::Reconnecting(new_attempt));
                        *shared_close.reconnect_attempts.borrow_mut() = new_attempt;

                        tracing::info!(
                            "Scheduling reconnect attempt {} in {}ms",
                            new_attempt,
                            delay
                        );

                        // Schedule reconnection via setTimeout
                        let shared_timer = shared_close.clone();
                        let callback = Closure::once(Box::new(move || {
                            *shared_timer.reconnect_timer_id.borrow_mut() = None;
                            setup_ws_connection(&shared_timer, &room_id);
                        }) as Box<dyn FnOnce()>);

                        let Some(window) = web_sys::window() else {
                            tracing::error!("No window for reconnect timer");
                            return;
                        };
                        if let Ok(timer_id) = window
                            .set_timeout_with_callback_and_timeout_and_arguments_0(
                                callback.as_ref().unchecked_ref(),
                                delay as i32,
                            )
                        {
                            *shared_close.reconnect_timer_id.borrow_mut() = Some(timer_id);
                        }
                        callback.forget();
                    }
                } else {
                    tracing::warn!(
                        "Max reconnection attempts ({}) reached",
                        MAX_RECONNECT_ATTEMPTS
                    );
                    (shared_close.on_state_change)(ConnectionState::Error);
                }
            }) as Box<dyn FnMut(JsValue)>);
            ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
            onclose.forget();

            *shared.socket.borrow_mut() = Some(ws);
        }
        Err(e) => {
            tracing::error!("Failed to create WebSocket: {:?}", e);
            (shared.on_state_change)(ConnectionState::Error);
        }
    }
}

// =============================================================================
// CollabManager
// =============================================================================

/// Collaboration manager for real-time strategy sync via WebSocket.
///
/// This is a plain struct with no Dioxus signals. Store it in `use_hook(|| ...)`
/// and wire the callbacks to update Dioxus signals in the component layer.
pub struct CollabManager {
    shared: Rc<SharedWsState>,
    last_cursor_send: Rc<RefCell<f64>>,
}

impl CollabManager {
    /// Create a new CollabManager.
    ///
    /// - `on_state_change`: called whenever the connection state changes.
    /// - `on_message`: called for each incoming `ServerMessage` from the server.
    pub fn new(
        on_state_change: impl Fn(ConnectionState) + 'static,
        on_message: impl Fn(ServerMessage) + 'static,
    ) -> Self {
        let shared = Rc::new(SharedWsState {
            socket: RefCell::new(None),
            reconnect_attempts: RefCell::new(0),
            current_room: RefCell::new(None),
            reconnect_timer_id: RefCell::new(None),
            intentional_disconnect: RefCell::new(false),
            on_state_change: Rc::new(on_state_change),
            on_message: Rc::new(on_message),
        });

        Self {
            shared,
            last_cursor_send: Rc::new(RefCell::new(0.0)),
        }
    }

    /// Connect to the collaboration server for the given strategy.
    pub fn connect(&self, strategy_id: &StrategyId) {
        *self.shared.current_room.borrow_mut() = Some(strategy_id.clone());
        *self.shared.intentional_disconnect.borrow_mut() = false;
        *self.shared.reconnect_attempts.borrow_mut() = 0;
        self.cancel_reconnect_timer();

        setup_ws_connection(&self.shared, strategy_id);
    }

    /// Disconnect from the server (intentional -- no reconnection).
    pub fn disconnect(&self) {
        *self.shared.intentional_disconnect.borrow_mut() = true;
        self.cancel_reconnect_timer();

        if let Some(socket) = self.shared.socket.borrow_mut().take() {
            let _ = socket.close();
        }
        (self.shared.on_state_change)(ConnectionState::Disconnected);
    }

    /// Manually retry connection after max attempts or error.
    pub fn retry(&self) {
        let room = self.shared.current_room.borrow().clone();
        if let Some(strategy_id) = room {
            *self.shared.reconnect_attempts.borrow_mut() = 0;
            *self.shared.intentional_disconnect.borrow_mut() = false;
            setup_ws_connection(&self.shared, &strategy_id);
        }
    }

    /// Send a message to the server.
    pub fn send(&self, message: ClientMessage) {
        if let Some(socket) = self.shared.socket.borrow().as_ref() {
            let request = WsRequest {
                request_id: None,
                message,
            };
            if let Ok(json) = serde_json::to_string(&request) {
                let _ = socket.send_with_str(&json);
            }
        }
    }

    /// Send cursor position (throttled to avoid flooding).
    ///
    /// Only sends if at least `CURSOR_THROTTLE_MS` have elapsed since the last
    /// cursor message.
    pub fn send_cursor(&self, position: Position) {
        let now = js_sys::Date::now();
        let last_send = *self.last_cursor_send.borrow();

        if now - last_send >= CURSOR_THROTTLE_MS as f64 {
            *self.last_cursor_send.borrow_mut() = now;
            self.send(ClientMessage::CursorMove { position });
        }
    }

    /// Cancel any pending reconnection timer.
    fn cancel_reconnect_timer(&self) {
        if let Some(timer_id) = self.shared.reconnect_timer_id.borrow_mut().take() {
            if let Some(window) = web_sys::window() {
                window.clear_timeout_with_handle(timer_id);
            }
        }
    }
}
