//! WebSocket client for Nostr relay communication (NIP-01, NIP-42, NIP-29).
//!
//! Mirrors the `CollabManager` pattern: plain Rust struct with callbacks, no
//! Dioxus signals internally. The component layer wraps it in signals.
//!
//! Connection flow:
//! 1. Member authenticates via Axum session
//! 2. `NostrRelayManager` opens WebSocket to strfry relay
//! 3. On AUTH challenge, POSTs challenge to Axum auth endpoint
//! 4. Server signs a fresh AUTH event bound to the challenge (NIP-42)
//! 5. Frontend sends the signed event to the relay
//! 6. Subscribes to NIP-29 group events

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{MessageEvent, Request, RequestCredentials, RequestInit, Response, WebSocket};

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
    Eose { subscription_id: String },
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
    /// The relay rejected NIP-42 sign-in. `message` is chat-banner text.
    AuthRejected { message: String },
    /// Relay sent a notice.
    Notice(String),
    /// Relay refused a subscription. `message` is chat-banner text.
    SubscriptionRefused { message: String },
}

// =============================================================================
// Reconnection helpers
// =============================================================================

fn reconnect_delay(attempt: u32) -> u32 {
    let delay = BASE_RECONNECT_DELAY_MS.saturating_mul(2u32.saturating_pow(attempt.min(5)));
    delay.min(MAX_RECONNECT_DELAY_MS)
}

/// NIP-42 machine-readable prefix on a `CLOSED` reason.
const AUTH_REQUIRED_PREFIX: &str = "auth-required:";

/// What to do with one relay `CLOSED` frame. Never closes the WebSocket.
#[derive(Clone, Debug, PartialEq, Eq)]
enum CloseOutcome {
    /// AUTH has not finished. Send this subscription after it succeeds.
    Queued,
    /// AUTH has finished. Send this subscription one time.
    Resend,
    /// Show this sentence in the chat error banner.
    Banner(String),
    /// The relay rejected NIP-42 sign-in. Show the banner and stop.
    SignInRejected(String),
    /// Sign-in already failed. Leave the socket and the banner alone.
    Held,
}

/// Result of the OK frame for the AUTH event we sent.
#[derive(Clone, Debug, PartialEq, Eq)]
enum AuthOkOutcome {
    /// AUTH succeeded. These subscription ids may be sent once.
    Accepted(Vec<String>),
    /// AUTH was rejected. Banner text for the chat widget.
    Rejected(String),
}

/// Per-connection bookkeeping for subscription `CLOSED` frames.
///
/// Once this relay has sent an AUTH challenge, a new socket does not put
/// `REQ` on the wire until NIP-42 AUTH succeeds. An `auth-required`
/// refusal is retried once per connection, so a second refusal cannot
/// spin the socket. A relay that never challenges is left alone.
#[derive(Debug, Default)]
struct RelayCloseGate {
    authed: bool,
    /// Sticky across reconnects. Set by an AUTH challenge or an
    /// `auth-required` CLOSED reason.
    relay_requires_auth: bool,
    /// We have sent an AUTH event and are waiting for its OK.
    auth_presented: bool,
    /// The relay rejected sign-in. Do not release subscriptions or reconnect.
    auth_rejected: bool,
    waiting: Vec<String>,
    resent: Vec<String>,
}

impl RelayCloseGate {
    fn reset_connection(&mut self) {
        let relay_requires_auth = self.relay_requires_auth;
        *self = Self {
            relay_requires_auth,
            ..Self::default()
        };
    }

    fn note_auth_challenge(&mut self) {
        self.relay_requires_auth = true;
    }

    fn note_auth_presented(&mut self) {
        self.auth_presented = true;
    }

    fn mark_rejected(&mut self) {
        self.auth_rejected = true;
        self.authed = false;
        self.waiting.clear();
    }

    /// `false` after a rejected sign-in, so a socket close does not reconnect.
    fn allows_reconnect(&self) -> bool {
        !self.auth_rejected
    }

    fn contains(ids: &[String], subscription_id: &str) -> bool {
        ids.iter().any(|id| id == subscription_id)
    }

    fn queue(&mut self, subscription_id: &str) {
        if Self::contains(&self.waiting, subscription_id)
            || Self::contains(&self.resent, subscription_id)
        {
            return;
        }
        self.waiting.push(subscription_id.to_string());
    }

    /// `true` means the caller must not send the REQ yet.
    fn hold_outgoing_req(&mut self, auth_configured: bool, subscription_id: &str) -> bool {
        if self.auth_rejected {
            return true;
        }
        if !auth_configured || !self.relay_requires_auth || self.authed {
            return false;
        }
        self.queue(subscription_id);
        true
    }

    /// `true` means stored subscriptions may be sent on this open socket.
    fn allow_open_resubscribe(
        &mut self,
        auth_configured: bool,
        subscription_ids: &[String],
    ) -> bool {
        if auth_configured && self.relay_requires_auth && !self.authed {
            for id in subscription_ids {
                self.queue(id);
            }
            return false;
        }
        true
    }

    fn on_auth_success(&mut self) -> Vec<String> {
        if self.auth_rejected {
            return Vec::new();
        }
        self.authed = true;
        let waiting = std::mem::take(&mut self.waiting);
        let mut send = Vec::new();
        for id in waiting {
            if Self::contains(&self.resent, &id) {
                continue;
            }
            self.resent.push(id.clone());
            send.push(id);
        }
        send
    }

    fn on_auth_ok(&mut self, accepted: bool, message: &str) -> AuthOkOutcome {
        if self.auth_rejected || !accepted {
            self.mark_rejected();
            return AuthOkOutcome::Rejected(sign_in_banner(message));
        }
        AuthOkOutcome::Accepted(self.on_auth_success())
    }

    /// `Some` when this NOTICE is the relay rejecting sign-in.
    fn on_notice(&mut self, message: &str) -> Option<String> {
        if !self.notice_or_closed_is_sign_in_failure(message) {
            return None;
        }
        self.mark_rejected();
        Some(sign_in_banner(message))
    }

    fn notice_or_closed_is_sign_in_failure(&self, reason: &str) -> bool {
        if self.authed || !self.auth_presented {
            return false;
        }
        is_sign_in_failure_reason(reason)
    }

    fn on_closed(&mut self, subscription_id: &str, reason: &str) -> CloseOutcome {
        if self.auth_rejected {
            if is_sign_in_failure_reason(reason) {
                return CloseOutcome::SignInRejected(sign_in_banner(reason));
            }
            // Sign-in already failed. Do not queue a resend or replace the banner.
            return CloseOutcome::Held;
        }
        if self.notice_or_closed_is_sign_in_failure(reason) {
            self.mark_rejected();
            return CloseOutcome::SignInRejected(sign_in_banner(reason));
        }
        if reason.starts_with(AUTH_REQUIRED_PREFIX) {
            self.relay_requires_auth = true;
            if Self::contains(&self.resent, subscription_id) {
                return CloseOutcome::Banner(relay_closed_banner(reason));
            }
            if self.authed {
                self.resent.push(subscription_id.to_string());
                self.waiting.retain(|id| id != subscription_id);
                return CloseOutcome::Resend;
            }
            self.queue(subscription_id);
            return CloseOutcome::Queued;
        }
        self.waiting.retain(|id| id != subscription_id);
        CloseOutcome::Banner(relay_closed_banner(reason))
    }
}

fn relay_closed_banner(reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        "The chat relay refused this channel.".to_string()
    } else {
        format!("The chat relay refused this channel: {reason}")
    }
}

fn sign_in_banner(reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        "The chat relay rejected sign-in.".to_string()
    } else {
        format!("The chat relay rejected sign-in: {reason}")
    }
}

/// CLOSED or NOTICE text that means the AUTH event was rejected.
///
/// A plain `auth-required:` hold ("AUTH required before REQ") is not a
/// rejection. That one still waits for a successful AUTH and resends once.
fn is_sign_in_failure_reason(reason: &str) -> bool {
    let lower = reason.trim().to_ascii_lowercase();
    if lower.starts_with("invalid:") || lower.starts_with("error:") {
        return true;
    }
    let Some(detail) = lower.strip_prefix("auth-required:") else {
        return false;
    };
    let detail = detail.trim();
    [
        "invalid",
        "fail",
        "rejected",
        "denied",
        "mismatch",
        "signature",
        "unauthorized",
        "bad",
    ]
    .iter()
    .any(|marker| detail.contains(marker))
}

fn warn_console(message: &str) {
    web_sys::console::warn_1(&JsValue::from_str(message));
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
    /// Endpoint for async NIP-42 challenge-response auth (e.g., "/api/chat/auth-token").
    auth_endpoint: RefCell<Option<String>>,
    subscriptions: RefCell<Vec<(String, Vec<NostrFilter>)>>,
    current_group: RefCell<Option<String>>,
    on_state_change: RefCell<Box<dyn FnMut(RelayConnectionState)>>,
    on_event: RefCell<Box<dyn FnMut(NostrRelayEvent)>>,
    /// NIP-42 bookkeeping for this socket. Reset on each new connection.
    auth_gate: RefCell<RelayCloseGate>,
    /// Event id of the AUTH frame just sent, so its OK is not a publish.
    pending_auth_event_id: RefCell<Option<String>>,
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
    shared.auth_gate.borrow_mut().reset_connection();
    *shared.pending_auth_event_id.borrow_mut() = None;

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

                // Re-subscribe after reconnect. Once this relay has asked for
                // NIP-42, wait until AUTH succeeds. A pre-auth REQ is what
                // it answers with CLOSED `auth-required:`, and repeating
                // that on every reconnect is the Authenticating/Reconnecting
                // loop.
                let subs = shared_open.subscriptions.borrow().clone();
                let ids: Vec<String> = subs.iter().map(|(id, _)| id.clone()).collect();
                let auth_configured = shared_open.auth_endpoint.borrow().is_some();
                let send_now = shared_open
                    .auth_gate
                    .borrow_mut()
                    .allow_open_resubscribe(auth_configured, &ids);
                if send_now
                    && !subs.is_empty()
                    && let Some(socket) = shared_open.socket.borrow().as_ref()
                {
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
                            // A bad frame is not a dead socket. Reconnect runs
                            // only from the WebSocket close handler below.
                            warn_console(&format!("Failed to parse relay message: {err}"));
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
            let onclose = Closure::wrap(Box::new(move |ev: JsValue| {
                // Transport close only. A NIP-01 CLOSED frame is handled in
                // `handle_relay_message` and must not reach this path.
                if let Ok(ev) = ev.dyn_into::<web_sys::CloseEvent>() {
                    warn_console(&format!(
                        "Relay WebSocket closed (code {}, reason {})",
                        ev.code(),
                        ev.reason()
                    ));
                }
                *shared_close.socket.borrow_mut() = None;

                if *shared_close.intentional_disconnect.borrow() {
                    shared_close.emit_state(RelayConnectionState::Disconnected);
                    return;
                }

                // A rejected sign-in stays on Error. Do not schedule another
                // socket or the page loops Authenticating/Reconnecting.
                if !shared_close.auth_gate.borrow().allows_reconnect() {
                    return;
                }

                let attempts = *shared_close.reconnect_attempts.borrow();
                if attempts < MAX_RECONNECT_ATTEMPTS {
                    let delay = reconnect_delay(attempts);
                    let new_attempt = attempts + 1;
                    shared_close.emit_state(RelayConnectionState::Reconnecting(new_attempt));
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
            if event.is_group_chat()
                && let Some(chat_msg) = ChatMessage::from_event(&event)
            {
                shared.emit_event(NostrRelayEvent::ChatMessage(chat_msg));
                return;
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
            let pending = shared.pending_auth_event_id.borrow().clone();
            if pending.as_deref() == Some(event_id.as_str()) {
                shared.pending_auth_event_id.borrow_mut().take();
                match shared.auth_gate.borrow_mut().on_auth_ok(accepted, &message) {
                    AuthOkOutcome::Accepted(ids) => {
                        for id in &ids {
                            resend_subscription(shared, id);
                        }
                        shared.emit_event(NostrRelayEvent::AuthSuccess);
                        shared.emit_state(RelayConnectionState::Ready);
                    }
                    AuthOkOutcome::Rejected(banner) => fail_sign_in(shared, banner),
                }
                return;
            }
            shared.emit_event(NostrRelayEvent::EventPublished {
                event_id,
                accepted,
                message,
            });
        }
        RelayMessage::Eose { subscription_id } => {
            shared.emit_event(NostrRelayEvent::Eose { subscription_id });
        }
        RelayMessage::Auth { challenge } => {
            shared.auth_gate.borrow_mut().note_auth_challenge();
            shared.emit_event(NostrRelayEvent::AuthChallenge);
            shared.emit_state(RelayConnectionState::Authenticating);

            // Async NIP-42: POST challenge to server, get freshly-signed auth event
            let auth_endpoint = shared.auth_endpoint.borrow().clone();
            let relay_url = shared.relay_url.borrow().clone().unwrap_or_default();
            if let Some(endpoint) = auth_endpoint {
                let shared_auth = shared.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    match fetch_auth_event(&endpoint, &relay_url, &challenge).await {
                        Ok(event) => {
                            shared_auth.auth_gate.borrow_mut().note_auth_presented();
                            *shared_auth.pending_auth_event_id.borrow_mut() =
                                Some(event.id.clone());
                            if let Some(socket) = shared_auth.socket.borrow().as_ref() {
                                let msg = ClientRelayMessage::Auth(event);
                                if let Ok(json) = msg.to_json() {
                                    let _ = socket.send_with_str(&json);
                                    tracing::info!("Presented challenge-bound NIP-42 AUTH event");
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!("NIP-42 auth failed: {e}");
                            shared_auth.emit_state(RelayConnectionState::Error);
                        }
                    }
                });
            } else {
                tracing::warn!("Relay sent AUTH challenge but no auth endpoint configured");
            }
        }
        RelayMessage::Notice(msg) => {
            if let Some(banner) = shared.auth_gate.borrow_mut().on_notice(&msg) {
                fail_sign_in(shared, banner);
                return;
            }
            tracing::info!("Relay notice: {}", msg);
            shared.emit_event(NostrRelayEvent::Notice(msg));
        }
        RelayMessage::Closed {
            subscription_id,
            message,
        } => {
            // CLOSED retires one subscription. Leave the WebSocket up.
            warn_console(&format!(
                "Relay CLOSED subscription {subscription_id}: {message}"
            ));
            let outcome = shared
                .auth_gate
                .borrow_mut()
                .on_closed(&subscription_id, &message);
            match outcome {
                CloseOutcome::Queued => {}
                CloseOutcome::Resend => resend_subscription(shared, &subscription_id),
                CloseOutcome::Banner(text) => {
                    shared.emit_event(NostrRelayEvent::SubscriptionRefused { message: text });
                }
                CloseOutcome::SignInRejected(text) => fail_sign_in(shared, text),
                CloseOutcome::Held => {}
            }
        }
    }
}

fn fail_sign_in(shared: &Rc<SharedRelayState>, banner: String) {
    warn_console(&banner);
    shared.auth_gate.borrow_mut().mark_rejected();
    if let Some(timer_id) = shared.reconnect_timer_id.borrow_mut().take()
        && let Some(window) = web_sys::window()
    {
        window.clear_timeout_with_handle(timer_id);
    }
    // Error first, then the banner event, so the sign-in sentence wins
    // over the generic connection-error copy.
    shared.emit_state(RelayConnectionState::Error);
    shared.emit_event(NostrRelayEvent::AuthRejected { message: banner });
}

fn resend_subscription(shared: &SharedRelayState, subscription_id: &str) {
    let filters = shared
        .subscriptions
        .borrow()
        .iter()
        .find(|(id, _)| id == subscription_id)
        .map(|(_, filters)| filters.clone());
    let Some(filters) = filters else {
        return;
    };
    let msg = ClientRelayMessage::Req {
        subscription_id: subscription_id.to_string(),
        filters,
    };
    if let Some(socket) = shared.socket.borrow().as_ref()
        && let Ok(json) = msg.to_json()
    {
        let _ = socket.send_with_str(&json);
    }
}

// =============================================================================
// NIP-42 Auth Fetch
// =============================================================================

/// Fetch a freshly-signed NIP-42 AUTH event from the server.
///
/// POSTs the relay challenge to the auth endpoint so the server signs an event
/// bound to this specific challenge, as required by NIP-42.
async fn fetch_auth_event(
    endpoint: &str,
    relay_url: &str,
    challenge: &str,
) -> Result<NostrEvent, String> {
    let body = serde_json::json!({
        "relay_url": relay_url,
        "challenge": challenge,
    });
    let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;

    let opts = RequestInit::new();
    opts.set_method("POST");
    opts.set_body(&JsValue::from_str(&body_str));
    opts.set_credentials(RequestCredentials::SameOrigin);

    let request = Request::new_with_str_and_init(endpoint, &opts).map_err(|e| format!("{e:?}"))?;
    request
        .headers()
        .set("Content-Type", "application/json")
        .map_err(|e| format!("{e:?}"))?;

    let window = web_sys::window().ok_or("no window")?;
    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("{e:?}"))?;
    let resp: Response = resp_value.dyn_into().map_err(|_| "response cast failed")?;

    if !resp.ok() {
        return Err(format!("auth endpoint returned {}", resp.status()));
    }

    let json = JsFuture::from(resp.json().map_err(|e| format!("{e:?}"))?)
        .await
        .map_err(|e| format!("{e:?}"))?;

    let parsed: serde_json::Value =
        serde_wasm_bindgen::from_value(json).map_err(|e| e.to_string())?;

    let event: NostrEvent = serde_json::from_value(
        parsed
            .get("auth_event")
            .cloned()
            .ok_or("missing auth_event in response")?,
    )
    .map_err(|e| e.to_string())?;

    Ok(event)
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
            auth_endpoint: RefCell::new(None),
            subscriptions: RefCell::new(Vec::new()),
            current_group: RefCell::new(None),
            on_state_change: RefCell::new(Box::new(on_state_change)),
            on_event: RefCell::new(Box::new(on_event)),
            auth_gate: RefCell::new(RelayCloseGate::default()),
            pending_auth_event_id: RefCell::new(None),
        });

        Self {
            shared,
            next_sub_id: Rc::new(RefCell::new(1)),
        }
    }

    /// Set the auth endpoint URL for NIP-42 challenge-response authentication.
    ///
    /// When the relay sends an AUTH challenge, the manager will POST the challenge
    /// to this endpoint and send the freshly-signed auth event back to the relay.
    pub fn set_auth_endpoint(&self, url: &str) {
        *self.shared.auth_endpoint.borrow_mut() = Some(url.to_string());
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

    /// Disconnect from the relay (intentional, no reconnection).
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
        if let ClientRelayMessage::Req {
            subscription_id, ..
        } = &message
        {
            let auth_configured = self.shared.auth_endpoint.borrow().is_some();
            if self
                .shared
                .auth_gate
                .borrow_mut()
                .hold_outgoing_req(auth_configured, subscription_id)
            {
                return;
            }
        }
        if let Some(socket) = self.shared.socket.borrow().as_ref()
            && let Ok(json) = message.to_json()
        {
            let _ = socket.send_with_str(&json);
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
        if let Some(timer_id) = self.shared.reconnect_timer_id.borrow_mut().take()
            && let Some(window) = web_sys::window()
        {
            window.clear_timeout_with_handle(timer_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn req_is_not_held_until_the_relay_asks_for_auth() {
        let mut gate = RelayCloseGate::default();
        assert!(!gate.hold_outgoing_req(true, "sub-1"));
        assert!(gate.allow_open_resubscribe(true, &["sub-1".into()]));
        gate.note_auth_challenge();
        assert!(gate.hold_outgoing_req(true, "sub-2"));
    }

    #[test]
    fn held_req_is_sent_once_when_auth_succeeds() {
        let mut gate = RelayCloseGate::default();
        gate.note_auth_challenge();
        assert!(gate.hold_outgoing_req(true, "sub-1"));
        assert!(gate.hold_outgoing_req(true, "sub-1"));
        assert_eq!(
            gate.on_closed("sub-1", "auth-required: still waiting"),
            CloseOutcome::Queued
        );
        assert_eq!(gate.on_auth_success(), vec!["sub-1".to_string()]);
        assert!(gate.on_auth_success().is_empty());
        assert!(!gate.hold_outgoing_req(true, "sub-2"));
    }

    #[test]
    fn auth_required_after_success_resends_once() {
        let mut gate = RelayCloseGate::default();
        assert!(gate.on_auth_success().is_empty());
        assert_eq!(
            gate.on_closed("sub-9", "auth-required: late"),
            CloseOutcome::Resend
        );
        assert_eq!(
            gate.on_closed("sub-9", "auth-required: late"),
            CloseOutcome::Banner("The chat relay refused this channel: auth-required: late".into())
        );
    }

    #[test]
    fn other_reason_is_a_banner_and_is_not_resent() {
        let mut gate = RelayCloseGate::default();
        gate.note_auth_challenge();
        assert!(gate.hold_outgoing_req(true, "sub-1"));
        assert_eq!(
            gate.on_closed("sub-1", "restricted: not a member"),
            CloseOutcome::Banner(
                "The chat relay refused this channel: restricted: not a member".into()
            )
        );
        assert!(gate.on_auth_success().is_empty());
    }

    #[test]
    fn auth_required_prefix_must_lead() {
        let mut gate = RelayCloseGate::default();
        let outcome = gate.on_closed("sub-1", "restricted: auth-required: no");
        assert_eq!(
            outcome,
            CloseOutcome::Banner(
                "The chat relay refused this channel: restricted: auth-required: no".into()
            )
        );
    }

    #[test]
    fn blank_reason_banner_stops_after_the_channel() {
        let mut gate = RelayCloseGate::default();
        assert_eq!(
            gate.on_closed("sub-1", "   "),
            CloseOutcome::Banner("The chat relay refused this channel.".into())
        );
    }

    #[test]
    fn open_waits_after_the_relay_has_asked_for_auth() {
        let mut gate = RelayCloseGate::default();
        gate.note_auth_challenge();
        gate.reset_connection();
        assert!(!gate.allow_open_resubscribe(true, &["keep".into()]));
        assert_eq!(gate.on_auth_success(), vec!["keep".to_string()]);
    }

    #[test]
    fn open_resends_immediately_without_auth_endpoint() {
        let mut gate = RelayCloseGate::default();
        assert!(gate.allow_open_resubscribe(false, &["keep".into()]));
        assert!(gate.on_auth_success().is_empty());
    }

    #[test]
    fn new_connection_holds_again() {
        let mut gate = RelayCloseGate::default();
        gate.note_auth_challenge();
        let _ = gate.on_auth_success();
        assert!(!gate.hold_outgoing_req(true, "sub-1"));
        gate.reset_connection();
        assert!(gate.hold_outgoing_req(true, "sub-1"));
    }

    #[test]
    fn rejected_sign_in_stays_in_error_and_does_not_reconnect() {
        let mut gate = RelayCloseGate::default();
        gate.note_auth_challenge();
        assert!(gate.hold_outgoing_req(true, "sub-1"));
        gate.note_auth_presented();

        assert_eq!(
            gate.on_auth_ok(false, "invalid: bad signature"),
            AuthOkOutcome::Rejected(
                "The chat relay rejected sign-in: invalid: bad signature".into()
            )
        );
        assert!(!gate.allows_reconnect());
        assert!(gate.hold_outgoing_req(true, "sub-2"));
        assert!(gate.on_auth_success().is_empty());
        assert_eq!(
            gate.on_auth_ok(true, ""),
            AuthOkOutcome::Rejected("The chat relay rejected sign-in.".into())
        );
        assert_eq!(
            gate.on_closed("sub-1", "auth-required: NIP-42 AUTH required before REQ"),
            CloseOutcome::Held
        );

        let mut closed = RelayCloseGate::default();
        closed.note_auth_presented();
        assert_eq!(
            closed.on_closed("sub-1", "auth-required: invalid signature"),
            CloseOutcome::SignInRejected(
                "The chat relay rejected sign-in: auth-required: invalid signature".into()
            )
        );
        assert!(!closed.allows_reconnect());
        assert!(closed.on_auth_success().is_empty());

        let mut notice = RelayCloseGate::default();
        notice.note_auth_challenge();
        assert_eq!(
            notice.on_notice("auth-required: NIP-42 AUTH required before REQ"),
            None
        );
        notice.note_auth_presented();
        assert_eq!(
            notice.on_notice("error: signature mismatch"),
            Some("The chat relay rejected sign-in: error: signature mismatch".into())
        );
        assert!(!notice.allows_reconnect());

        let mut still_waiting = RelayCloseGate::default();
        still_waiting.note_auth_presented();
        assert_eq!(
            still_waiting.on_closed("sub-1", "auth-required: NIP-42 AUTH required before REQ"),
            CloseOutcome::Queued
        );
        assert!(still_waiting.allows_reconnect());
    }
}
