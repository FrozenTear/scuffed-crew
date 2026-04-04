//! WebSocket client for Nostr relay communication (NIP-01, NIP-42, NIP-29).
//!
//! Mirrors the `CollabManager` pattern: plain Rust struct with callbacks, no
//! Dioxus signals internally. The component layer wraps it in signals.
//!
//! Connection flow:
//! 1. Member authenticates via Axum session
//! 2. Frontend fetches NIP-42 auth token from Axum
//! 3. `NostrRelayManager` opens WebSocket to strfry relay
//! 4. On AUTH challenge, presents the pre-signed auth event
//! 5. Subscribes to NIP-29 group events

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, WebSocket};

use scuffed_types::nostr::{
    ChatMessage, ClientRelayMessage, NostrEvent, NostrFilter, RelayMessage,
};

// =============================================================================
// Constants
// =============================================================================

/// Maximum reconnection attempts before giving up.
const MAX_RECONNECT_ATTEMPTS: u32 = 10;

/// Base delay for reconnection backoff (milliseconds).
const BASE_RECONNECT_DELAY_MS: u32 = 1000;

/// Maximum reconnection delay (milliseconds).
const MAX_RECONNECT_DELAY_MS: u32 = 30_000;

// =============================================================================
// RelayConnectionState
// =============================================================================

/// Relay WebSocket connection state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RelayConnectionState {
    Disconnected,
    Connecting,
    Connected,
    /// Connected to WebSocket, awaiting NIP-42 AUTH completion.
    Authenticating,
    /// Fully connected and authenticated.
    Ready,
    Reconnecting(u32),
    Error,
}

impl RelayConnectionState {
    pub fn is_ready(&self) -> bool {
        matches!(self, RelayConnectionState::Ready)
    }

    pub fn is_connected(&self) -> bool {
        matches!(
            self,
            RelayConnectionState::Connected
                | RelayConnectionState::Authenticating
                | RelayConnectionState::Ready
        )
    }

    pub fn is_reconnecting(&self) -> bool {
        matches!(self, RelayConnectionState::Reconnecting(_))
    }

    pub fn display_text(&self) -> &'static str {
        match self {
            RelayConnectionState::Disconnected => "Disconnected",
            RelayConnectionState::Connecting => "Connecting...",
            RelayConnectionState::Connected => "Connected",
            RelayConnectionState::Authenticating => "Authenticating...",
            RelayConnectionState::Ready => "Ready",
            RelayConnectionState::Reconnecting(_) => "Reconnecting...",
            RelayConnectionState::Error => "Connection error",
        }
    }
}

// =============================================================================
// Relay Events (from manager to UI layer)
// =============================================================================

/// Events emitted by the relay manager to the UI layer.
#[derive(Clone, Debug)]
pub enum NostrRelayEvent {
    /// A chat message was received.
    ChatMessage(ChatMessage),
    /// A raw event was received (for subscriptions the UI handles directly).
    RawEvent {
        subscription_id: String,
        event: NostrEvent,
    },
    /// End of stored events for a subscription.
    Eose {
        subscription_id: String,
    },
    /// An event we published was acknowledged.
    EventPublished {
        event_id: String,
        accepted: bool,
        message: String,
    },
    /// NIP-42 AUTH challenge received (manager handles this automatically
    /// if an auth event is configured, but UI may want to know).
    AuthChallenge,
    /// Authentication completed successfully.
    AuthSuccess,
    /// Relay sent a notice.
    Notice(String),
}

// =============================================================================
// Reconnection helpers
// =============================================================================

fn reconnect_delay(attempt: u32) -> u32 {
    let delay = BASE_RECONNECT_DELAY_MS.saturating_mul(2u32.saturating_pow(attempt.min(5)));
    delay.min(MAX_RECONNECT_DELAY_MS)
}

// =============================================================================
// SharedRelayState
// =============================================================================

/// Shared state for the relay WebSocket, accessible from JS closures.
///
/// Callbacks use `RefCell<Box<dyn FnMut>>` because Dioxus 0.7 `Signal::set()`
/// takes `&mut self`, making component-layer closures `FnMut`.
struct SharedRelayState {
    socket: RefCell<Option<WebSocket>>,
    reconnect_attempts: RefCell<u32>,
    reconnect_timer_id: RefCell<Option<i32>>,
    intentional_disconnect: RefCell<bool>,
    relay_url: RefCell<Option<String>>,
    auth_event: RefCell<Option<NostrEvent>>,
    subscriptions: RefCell<Vec<(String, Vec<NostrFilter>)>>,
    current_group: RefCell<Option<String>>,
    on_state_change: RefCell<Box<dyn FnMut(RelayConnectionState)>>,
    on_event: RefCell<Box<dyn FnMut(NostrRelayEvent)>>,
}

impl SharedRelayState {
    fn emit_state(&self, state: RelayConnectionState) {
        (self.on_state_change.borrow_mut())(state);
    }

    fn emit_event(&self, event: NostrRelayEvent) {
        (self.on_event.borrow_mut())(event);
    }
}

// =============================================================================
// WebSocket setup
// =============================================================================

fn setup_relay_connection(shared: &Rc<SharedRelayState>) {
    let url = match shared.relay_url.borrow().clone() {
        Some(url) => url,
        None => {
            tracing::error!("No relay URL configured");
            shared.emit_state(RelayConnectionState::Error);
            return;
        }
    };

    shared.emit_state(RelayConnectionState::Connecting);

    match WebSocket::new(&url) {
        Ok(ws) => {
            ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

            // -- On open --
            let shared_open = shared.clone();
            let onopen = Closure::wrap(Box::new(move |_: JsValue| {
                tracing::info!("Relay WebSocket connected");
                shared_open.emit_state(RelayConnectionState::Connected);
                *shared_open.reconnect_attempts.borrow_mut() = 0;

                // Re-subscribe to active subscriptions after reconnect
                let subs = shared_open.subscriptions.borrow().clone();
                if !subs.is_empty() {
                    if let Some(socket) = shared_open.socket.borrow().as_ref() {
                        for (sub_id, filters) in &subs {
                            let msg = ClientRelayMessage::Req {
                                subscription_id: sub_id.clone(),
                                filters: filters.clone(),
                            };
                            if let Ok(json) = msg.to_json() {
                                let _ = socket.send_with_str(&json);
                            }
                        }
                    }
                }
            }) as Box<dyn FnMut(JsValue)>);
            ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
            onopen.forget();

            // -- On message --
            let shared_msg = shared.clone();
            let onmessage = Closure::wrap(Box::new(move |e: MessageEvent| {
                if let Ok(text) = e.data().dyn_into::<js_sys::JsString>() {
                    let text: String = text.into();
                    match RelayMessage::from_json(&text) {
                        Ok(msg) => handle_relay_message(&shared_msg, msg),
                        Err(err) => {
                            tracing::warn!("Failed to parse relay message: {}", err);
                        }
                    }
                }
            }) as Box<dyn FnMut(MessageEvent)>);
            ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
            onmessage.forget();

            // -- On error --
            let shared_err = shared.clone();
            let onerror = Closure::wrap(Box::new(move |_: JsValue| {
                tracing::error!("Relay WebSocket error");
                shared_err.emit_state(RelayConnectionState::Error);
            }) as Box<dyn FnMut(JsValue)>);
            ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
            onerror.forget();

            // -- On close --
            let shared_close = shared.clone();
            let onclose = Closure::wrap(Box::new(move |_: JsValue| {
                tracing::info!("Relay WebSocket closed");
                *shared_close.socket.borrow_mut() = None;

                if *shared_close.intentional_disconnect.borrow() {
                    shared_close.emit_state(RelayConnectionState::Disconnected);
                    return;
                }

                let attempts = *shared_close.reconnect_attempts.borrow();
                if attempts < MAX_RECONNECT_ATTEMPTS {
                    let delay = reconnect_delay(attempts);
                    let new_attempt = attempts + 1;
                    shared_close
                        .emit_state(RelayConnectionState::Reconnecting(new_attempt));
                    *shared_close.reconnect_attempts.borrow_mut() = new_attempt;

                    tracing::info!(
                        "Scheduling relay reconnect attempt {} in {}ms",
                        new_attempt,
                        delay
                    );

                    let shared_timer = shared_close.clone();
                    let callback = Closure::once(Box::new(move || {
                        *shared_timer.reconnect_timer_id.borrow_mut() = None;
                        setup_relay_connection(&shared_timer);
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
                } else {
                    tracing::warn!(
                        "Max relay reconnection attempts ({}) reached",
                        MAX_RECONNECT_ATTEMPTS
                    );
                    shared_close.emit_state(RelayConnectionState::Error);
                }
            }) as Box<dyn FnMut(JsValue)>);
            ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
            onclose.forget();

            *shared.socket.borrow_mut() = Some(ws);
        }
        Err(e) => {
            tracing::error!("Failed to create relay WebSocket: {:?}", e);
            shared.emit_state(RelayConnectionState::Error);
        }
    }
}

/// Handle a parsed relay message.
fn handle_relay_message(shared: &Rc<SharedRelayState>, msg: RelayMessage) {
    match msg {
        RelayMessage::Event {
            subscription_id,
            event,
        } => {
            if event.is_group_chat() {
                if let Some(chat_msg) = ChatMessage::from_event(&event) {
                    shared.emit_event(NostrRelayEvent::ChatMessage(chat_msg));
                    return;
                }
            }
            shared.emit_event(NostrRelayEvent::RawEvent {
                subscription_id,
                event,
            });
        }
        RelayMessage::Ok {
            event_id,
            accepted,
            message,
        } => {
            shared.emit_event(NostrRelayEvent::EventPublished {
                event_id,
                accepted,
                message,
            });
        }
        RelayMessage::Eose { subscription_id } => {
            shared.emit_event(NostrRelayEvent::Eose { subscription_id });
        }
        RelayMessage::Auth { challenge: _ } => {
            shared.emit_event(NostrRelayEvent::AuthChallenge);
            shared.emit_state(RelayConnectionState::Authenticating);

            // Auto-present auth event if we have one
            let auth_event = shared.auth_event.borrow().clone();
            if let Some(event) = auth_event {
                if let Some(socket) = shared.socket.borrow().as_ref() {
                    let msg = ClientRelayMessage::Auth(event);
                    if let Ok(json) = msg.to_json() {
                        let _ = socket.send_with_str(&json);
                        tracing::info!("Presented NIP-42 AUTH event to relay");
                    }
                }
            } else {
                tracing::warn!("Relay sent AUTH challenge but no auth event configured");
            }
        }
        RelayMessage::Notice(msg) => {
            tracing::info!("Relay notice: {}", msg);
            shared.emit_event(NostrRelayEvent::Notice(msg));
        }
    }
}

// =============================================================================
// NostrRelayManager
// =============================================================================

/// Manages WebSocket connection to a Nostr relay for the chat widget.
///
/// Plain struct with no Dioxus signals. Use in `use_hook(|| ...)` and wire
/// callbacks to update Dioxus signals.
///
/// Clone is cheap (Rc pointers).
#[derive(Clone)]
pub struct NostrRelayManager {
    shared: Rc<SharedRelayState>,
    next_sub_id: Rc<RefCell<u32>>,
}

impl NostrRelayManager {
    /// Create a new relay manager.
    ///
    /// - `on_state_change`: called when connection state changes.
    /// - `on_event`: called for incoming relay events.
    pub fn new(
        on_state_change: impl FnMut(RelayConnectionState) + 'static,
        on_event: impl FnMut(NostrRelayEvent) + 'static,
    ) -> Self {
        let shared = Rc::new(SharedRelayState {
            socket: RefCell::new(None),
            reconnect_attempts: RefCell::new(0),
            reconnect_timer_id: RefCell::new(None),
            intentional_disconnect: RefCell::new(false),
            relay_url: RefCell::new(None),
            auth_event: RefCell::new(None),
            subscriptions: RefCell::new(Vec::new()),
            current_group: RefCell::new(None),
            on_state_change: RefCell::new(Box::new(on_state_change)),
            on_event: RefCell::new(Box::new(on_event)),
        });

        Self {
            shared,
            next_sub_id: Rc::new(RefCell::new(1)),
        }
    }

    /// Set the NIP-42 auth event to present on AUTH challenge.
    pub fn set_auth_event(&self, event: NostrEvent) {
        *self.shared.auth_event.borrow_mut() = Some(event);
    }

    /// Connect to a Nostr relay.
    pub fn connect(&self, relay_url: &str) {
        *self.shared.relay_url.borrow_mut() = Some(relay_url.to_string());
        *self.shared.intentional_disconnect.borrow_mut() = false;
        *self.shared.reconnect_attempts.borrow_mut() = 0;
        self.cancel_reconnect_timer();

        setup_relay_connection(&self.shared);
    }

    /// Connect to the relay using the current page's origin (same-host relay).
    ///
    /// Constructs `ws[s]://<host>/relay` based on the current page protocol.
    pub fn connect_same_origin(&self, path: &str) {
        let Some(window) = web_sys::window() else {
            tracing::error!("No browser window");
            self.shared.emit_state(RelayConnectionState::Error);
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
                tracing::error!("Failed to read location.host");
                self.shared.emit_state(RelayConnectionState::Error);
                return;
            }
        };
        let url = format!("{}//{}{}", protocol, host, path);
        self.connect(&url);
    }

    /// Disconnect from the relay (intentional — no reconnection).
    pub fn disconnect(&self) {
        *self.shared.intentional_disconnect.borrow_mut() = true;
        self.cancel_reconnect_timer();

        // Close all subscriptions first
        if let Some(socket) = self.shared.socket.borrow().as_ref() {
            for (sub_id, _) in self.shared.subscriptions.borrow().iter() {
                let msg = ClientRelayMessage::Close(sub_id.clone());
                if let Ok(json) = msg.to_json() {
                    let _ = socket.send_with_str(&json);
                }
            }
        }
        self.shared.subscriptions.borrow_mut().clear();

        if let Some(socket) = self.shared.socket.borrow_mut().take() {
            let _ = socket.close();
        }
        self.shared.emit_state(RelayConnectionState::Disconnected);
    }

    /// Manually retry connection after error.
    pub fn retry(&self) {
        *self.shared.reconnect_attempts.borrow_mut() = 0;
        *self.shared.intentional_disconnect.borrow_mut() = false;
        setup_relay_connection(&self.shared);
    }

    /// Subscribe to NIP-29 group chat messages.
    ///
    /// Returns the subscription ID.
    pub fn subscribe_group(&self, group_id: &str, message_limit: Option<usize>) -> String {
        let sub_id = self.next_subscription_id("group");
        *self.shared.current_group.borrow_mut() = Some(group_id.to_string());

        let filters = vec![NostrFilter::group_chat(group_id, message_limit)];

        self.shared
            .subscriptions
            .borrow_mut()
            .push((sub_id.clone(), filters.clone()));

        self.send_raw(ClientRelayMessage::Req {
            subscription_id: sub_id.clone(),
            filters,
        });

        sub_id
    }

    /// Subscribe to NIP-29 group metadata.
    ///
    /// Returns the subscription ID.
    pub fn subscribe_group_metadata(&self, group_id: &str) -> String {
        let sub_id = self.next_subscription_id("meta");
        let filters = vec![NostrFilter::group_metadata(group_id)];

        self.shared
            .subscriptions
            .borrow_mut()
            .push((sub_id.clone(), filters.clone()));

        self.send_raw(ClientRelayMessage::Req {
            subscription_id: sub_id.clone(),
            filters,
        });

        sub_id
    }

    /// Subscribe with custom filters.
    ///
    /// Returns the subscription ID.
    pub fn subscribe(&self, prefix: &str, filters: Vec<NostrFilter>) -> String {
        let sub_id = self.next_subscription_id(prefix);

        self.shared
            .subscriptions
            .borrow_mut()
            .push((sub_id.clone(), filters.clone()));

        self.send_raw(ClientRelayMessage::Req {
            subscription_id: sub_id.clone(),
            filters,
        });

        sub_id
    }

    /// Close a subscription.
    pub fn unsubscribe(&self, subscription_id: &str) {
        self.shared
            .subscriptions
            .borrow_mut()
            .retain(|(id, _)| id != subscription_id);

        self.send_raw(ClientRelayMessage::Close(subscription_id.to_string()));
    }

    /// Publish a NIP-29 group chat message event.
    pub fn publish_event(&self, event: NostrEvent) {
        self.send_raw(ClientRelayMessage::Event(event));
    }

    /// Send a raw client relay message.
    fn send_raw(&self, message: ClientRelayMessage) {
        if let Some(socket) = self.shared.socket.borrow().as_ref() {
            if let Ok(json) = message.to_json() {
                let _ = socket.send_with_str(&json);
            }
        }
    }

    /// Generate a unique subscription ID.
    fn next_subscription_id(&self, prefix: &str) -> String {
        let mut counter = self.next_sub_id.borrow_mut();
        let id = format!("scuffed-{}-{}", prefix, *counter);
        *counter += 1;
        id
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
