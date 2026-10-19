//! Main expandable chat widget — sidebar/panel layout.
//!
//! Wires F-API-003/004: discovered `TeamChannel` group_ids, team (public)
//! sends via `POST /api/chat/send-public`, and send-encrypted / decrypt on
//! officer channels.

use dioxus::prelude::*;
use scuffed_api_client::{ApiClient, ClientError};
use scuffed_types::nostr::{ChatMessage, NostrEvent, NostrFilter, event_kinds};
use scuffed_types::{
    AuthTokenRequest, AuthTokenResponse, DecryptMessageRequest, DecryptMessageResponse, GroupType,
    MeResponse, SendEncryptedRequest, SendEncryptedResponse, SendPublicRequest, SendPublicResponse,
    TeamChannel, chat_api_error_copy,
};

use crate::components::chat::encrypted_chat::{EncryptedChat, KeyMode};
use crate::components::{Toast, use_toast};
use crate::state::RelayAvailability;
use crate::state::nostr::{NostrRelayEvent, NostrRelayManager, RelayConnectionState};
use crate::state::use_relay_availability_state;

use super::compose_box::{ComposeAttempt, ComposeBox};
use super::message_list::MessageList;
use super::relay_status::RelayStatus;

const CHAT_WIDGET_CSS: &str = r#"
.chat-widget {
    position: fixed;
    bottom: 0;
    right: 0;
    width: 360px;
    max-height: 520px;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-bottom: none;
    border-radius: 12px 12px 0 0;
    box-shadow: 0 -4px 24px var(--overlay);
    z-index: 1000;
    transition: transform 0.25s ease, opacity 0.2s ease;
    overflow: hidden;
}

.chat-widget--collapsed {
    max-height: 44px;
    cursor: pointer;
}

.chat-widget--collapsed .chat-widget__body {
    display: none;
}

.chat-widget--embedded {
    position: relative;
    width: 100%;
    max-height: none;
    height: min(70vh, 640px);
    border-radius: 12px;
    border-bottom: 1px solid var(--border);
    box-shadow: none;
    z-index: auto;
}

.chat-widget__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    cursor: pointer;
    user-select: none;
    min-height: 44px;
}

.chat-widget--embedded .chat-widget__header {
    cursor: default;
}

.chat-widget__header-left {
    display: flex;
    align-items: center;
    gap: 8px;
}

.chat-widget__title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
}

.chat-widget__header-right {
    display: flex;
    align-items: center;
    gap: 6px;
}

.chat-widget__toggle {
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    background: transparent;
    color: var(--text-2);
    cursor: pointer;
    border-radius: 4px;
    font-size: 16px;
    transition: color 0.15s ease, background-color 0.15s ease;
}

.chat-widget__toggle:hover {
    color: var(--text);
    background: var(--surface-2);
}

.chat-widget__body {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    height: 420px;
}

.chat-widget--embedded .chat-widget__body {
    height: auto;
}

.chat-widget__channel-bar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    scrollbar-width: none;
}

.chat-widget__channel-bar::-webkit-scrollbar {
    display: none;
}

.chat-widget__channel {
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: transparent;
    color: var(--text-2);
    font-size: 11px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: all 0.15s ease;
}

.chat-widget__channel:hover {
    border-color: var(--accent);
    color: var(--text);
}

.chat-widget__channel--active {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent);
}

.chat-widget__error {
    margin: 8px 12px 0;
    padding: 8px 10px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--danger) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    color: var(--danger);
    font-size: 12px;
    line-height: 1.4;
}

.chat-widget__empty {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-3);
    font-size: 13px;
    padding: 2rem;
    text-align: center;
}

@media (max-width: 480px) {
    .chat-widget {
        width: 100%;
        border-radius: 0;
        border: none;
        border-top: 1px solid var(--border);
    }
    .chat-widget--embedded {
        border: 1px solid var(--border);
        border-radius: 12px;
    }
}
"#;

fn first_active_group(channels: &[TeamChannel]) -> Option<String> {
    channels
        .iter()
        .find(|c| c.is_active)
        .map(|c| c.group_id.clone())
}

fn channel_by_group<'a>(channels: &'a [TeamChannel], group_id: &str) -> Option<&'a TeamChannel> {
    channels.iter().find(|c| c.group_id == group_id)
}

fn auth_relay_url(channel_relay: &str) -> String {
    if channel_relay.starts_with("ws://")
        || channel_relay.starts_with("wss://")
        || channel_relay.starts_with("http://")
        || channel_relay.starts_with("https://")
    {
        return channel_relay.to_string();
    }
    web_sys::window()
        .and_then(|w| w.location().origin().ok())
        .map(|origin| {
            if channel_relay.starts_with('/') {
                format!("{origin}{channel_relay}")
            } else {
                format!("{origin}/relay")
            }
        })
        .unwrap_or_else(|| "/relay".into())
}

/// Where the widget would open a socket. `None` means do not connect.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RelayDial {
    Absolute(String),
    SameOrigin(String),
}

/// No dial until health says the relay is configured. An active channel with
/// an empty or relative URL used to become `wss://<host>/relay`.
fn relay_dial(availability: RelayAvailability, channels: &[TeamChannel]) -> Option<RelayDial> {
    if !availability.allows_connection() {
        return None;
    }
    let url = channels
        .iter()
        .find(|channel| channel.is_active)
        .map(|channel| channel.relay_url.as_str())?;
    if url.starts_with("ws://") || url.starts_with("wss://") {
        Some(RelayDial::Absolute(url.to_string()))
    } else if url.starts_with('/') {
        Some(RelayDial::SameOrigin(url.to_string()))
    } else {
        Some(RelayDial::SameOrigin("/relay".to_string()))
    }
}

fn connect_relay(relay: &NostrRelayManager, dial: &RelayDial) {
    match dial {
        RelayDial::Absolute(url) => relay.connect(url),
        RelayDial::SameOrigin(path) => relay.connect_same_origin(path),
    }
}

/// `Authenticating` is excluded so a REQ is not sent while NIP-42 is in
/// flight. `Connected` still subscribes for a relay that never challenges.
fn subscription_allowed(state: RelayConnectionState) -> bool {
    matches!(
        state,
        RelayConnectionState::Connected | RelayConnectionState::Ready
    )
}

async fn provision_auth_token(relay_url: &str) -> Result<AuthTokenResponse, String> {
    let client = ApiClient::web();
    client
        .post_json::<_, AuthTokenResponse>(
            "/api/chat/auth-token",
            &AuthTokenRequest {
                relay_url: relay_url.to_string(),
                challenge: None,
            },
        )
        .await
        .map_err(|e| match e {
            ClientError::Http {
                status,
                body,
                retry_after: _,
            } => chat_api_error_copy(status, &body),
            other => other.to_string(),
        })
}

async fn send_officer_message(
    group_id: &str,
    content: &str,
    relay_url: &str,
) -> Result<SendEncryptedResponse, String> {
    let client = ApiClient::web();
    let req = SendEncryptedRequest {
        group_id: group_id.to_string(),
        content: content.to_string(),
        reply_to: None,
    };
    match client
        .post_json::<_, SendEncryptedResponse>("/api/chat/send-encrypted", &req)
        .await
    {
        Ok(r) => Ok(r),
        Err(ClientError::Http {
            status: 400,
            body,
            retry_after: _,
        }) => {
            if body.contains("External key") {
                return Err(chat_api_error_copy(400, &body));
            }
            provision_auth_token(relay_url).await?;
            match client
                .post_json::<_, SendEncryptedResponse>("/api/chat/send-encrypted", &req)
                .await
            {
                Ok(r) => Ok(r),
                Err(ClientError::Http {
                    status,
                    body,
                    retry_after: _,
                }) => Err(chat_api_error_copy(status, &body)),
                Err(e) => Err(e.to_string()),
            }
        }
        Err(ClientError::Http {
            status,
            body,
            retry_after: _,
        }) => Err(chat_api_error_copy(status, &body)),
        Err(e) => Err(e.to_string()),
    }
}

async fn decrypt_gift_wrap(event: &NostrEvent, relay_url: &str) -> Result<ChatMessage, String> {
    let client = ApiClient::web();
    let req = DecryptMessageRequest {
        event_json: serde_json::to_string(event).map_err(|e| e.to_string())?,
    };
    let resp = match client
        .post_json::<_, DecryptMessageResponse>("/api/chat/decrypt", &req)
        .await
    {
        Ok(r) => r,
        Err(ClientError::Http {
            status: 400,
            body,
            retry_after: _,
        }) => {
            if body.contains("External key") {
                return Err(chat_api_error_copy(400, &body));
            }
            let _ = provision_auth_token(relay_url).await;
            client
                .post_json::<_, DecryptMessageResponse>("/api/chat/decrypt", &req)
                .await
                .map_err(|e| match e {
                    ClientError::Http {
                        status,
                        body,
                        retry_after: _,
                    } => chat_api_error_copy(status, &body),
                    other => other.to_string(),
                })?
        }
        Err(ClientError::Http {
            status,
            body,
            retry_after: _,
        }) => return Err(chat_api_error_copy(status, &body)),
        Err(e) => return Err(e.to_string()),
    };
    let group_id = resp
        .tags
        .iter()
        .find(|t| t.first().map(|s| s.as_str()) == Some("h"))
        .and_then(|t| t.get(1))
        .cloned()
        .unwrap_or_default();
    let reply_to = resp
        .tags
        .iter()
        .find(|t| t.first().map(|s| s.as_str()) == Some("e"))
        .and_then(|t| t.get(1))
        .cloned();
    Ok(ChatMessage {
        id: event.id.clone(),
        pubkey: resp.sender_pubkey,
        display_name: None,
        avatar_url: None,
        content: resp.content,
        created_at: resp.created_at,
        group_id,
        encrypted: true,
        reply_to,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicSendError {
    message: String,
    retry_after_secs: Option<u64>,
}

const RATE_LIMIT_LATER: &str = "Too many messages. Try again later.";

fn rate_limit_copy(secs: u64) -> String {
    if secs == 1 {
        "Too many messages. Try again in 1 second.".to_string()
    } else {
        format!("Too many messages. Try again in {secs} seconds.")
    }
}

/// Whole seconds left until `until_ms`, rounded up. Zero once `now_ms` catches up.
fn remaining_hold_secs(until_ms: f64, now_ms: f64) -> u64 {
    if !until_ms.is_finite() || !now_ms.is_finite() || until_ms <= now_ms {
        return 0;
    }
    let secs = ((until_ms - now_ms) / 1000.0).ceil();
    if !secs.is_finite() || secs >= u64::MAX as f64 {
        u64::MAX
    } else {
        secs as u64
    }
}

/// Plain copy for `POST /api/chat/send-public` failures.
///
/// 429 uses the JSON `retry_after` (then the `Retry-After` header) so the
/// composer can count down. Waits outside 1..=3600 stay a static message.
fn public_send_failure(
    status: u16,
    body: &str,
    header_retry_after: Option<u64>,
) -> PublicSendError {
    if status == 429 {
        let secs = scuffed_types::json_retry_after(body).or(header_retry_after);
        return match secs {
            Some(secs) => PublicSendError {
                message: rate_limit_copy(secs),
                retry_after_secs: Some(secs),
            },
            None => PublicSendError {
                message: RATE_LIMIT_LATER.to_string(),
                retry_after_secs: None,
            },
        };
    }
    let message = match status {
        400 if body.contains("too long") => "Message is too long.",
        400 if body.contains("Message content must not be empty") => {
            "Write a message before sending."
        }
        400 => "That message couldn't be sent.",
        401 => "Sign in to send a message.",
        403 => "You are not allowed in this channel.",
        404 => "Channel not found.",
        502 | 503 => "Couldn't reach the chat server.",
        _ => "Something went wrong sending that message.",
    };
    PublicSendError {
        message: message.to_string(),
        retry_after_secs: None,
    }
}

fn public_send_client_error(err: ClientError) -> PublicSendError {
    match err {
        ClientError::Http {
            status,
            body,
            retry_after,
        } => public_send_failure(status, &body, retry_after),
        ClientError::Network(_) => PublicSendError {
            message: "Couldn't reach the chat server.".into(),
            retry_after_secs: None,
        },
        ClientError::Deserialize(_) => PublicSendError {
            message: "Something went wrong sending that message.".into(),
            retry_after_secs: None,
        },
    }
}

fn needs_key_provision(body: &str) -> bool {
    body.contains("No Nostr keys") || body.contains("Nostr keys provisioned")
}

async fn post_public_message(
    client: &ApiClient,
    req: &SendPublicRequest,
) -> Result<SendPublicResponse, PublicSendError> {
    client
        .post_json::<_, SendPublicResponse>("/api/chat/send-public", req)
        .await
        .map_err(public_send_client_error)
}

/// Publish a team channel message with the session cookie.
///
/// A member with no server key yet gets one provision attempt, the same
/// recovery officer send already uses, then one retry. The relay echo of the
/// returned event id is ignored by [`push_unique`].
async fn send_public_message(
    group_id: &str,
    content: &str,
    relay_url: &str,
) -> Result<SendPublicResponse, PublicSendError> {
    let client = ApiClient::web();
    let req = SendPublicRequest {
        group_id: group_id.to_string(),
        content: content.to_string(),
    };
    match client
        .post_json::<_, SendPublicResponse>("/api/chat/send-public", &req)
        .await
    {
        Ok(resp) => Ok(resp),
        Err(ClientError::Http {
            status: 400, body, ..
        }) if needs_key_provision(&body) => {
            if provision_auth_token(relay_url).await.is_err() {
                return Err(public_send_failure(400, &body, None));
            }
            post_public_message(&client, &req).await
        }
        Err(other) => Err(public_send_client_error(other)),
    }
}

fn push_unique(messages: &mut Vec<ChatMessage>, msg: ChatMessage) {
    if !messages.iter().any(|m| m.id == msg.id) {
        messages.push(msg);
    }
}

fn key_mode_from_me(me: Option<&MeResponse>) -> KeyMode {
    match me
        .and_then(|m| m.member.as_ref())
        .and_then(|m| m.nostr_key_mode.as_deref())
    {
        Some("server_managed") => KeyMode::ServerManaged,
        Some("external") => KeyMode::External,
        _ => KeyMode::None,
    }
}

/// Top-level expandable chat widget.
///
/// Manages its own relay connection lifecycle via `NostrRelayManager`.
#[component]
pub fn ChatWidget(
    /// Discovered team channels (`GET /api/teams/:id/channels`). Use returned group_ids only.
    channels: Vec<TeamChannel>,
    /// Page layout instead of the floating overlay.
    #[props(default = false)]
    embedded: bool,
) -> Element {
    let mut toast = use_toast();
    let mut expanded = use_signal(|| true);
    let mut connection_state = use_signal(|| RelayConnectionState::Disconnected);
    let mut messages = use_signal(Vec::<ChatMessage>::new);
    let mut active_group = use_signal(|| first_active_group(&channels));
    let mut current_sub_id = use_signal(|| Option::<String>::None);
    let mut wrap_sub_id = use_signal(|| Option::<String>::None);
    let mut loading = use_signal(|| false);
    let mut panel_error = use_signal(|| Option::<String>::None);
    let mut sending = use_signal(|| false);
    let mut send_hold_until_ms = use_signal(|| 0.0);
    let mut hold_tick = use_signal(|| 0u32);
    let _hold_timer = use_future(move || async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(1_000).await;
            hold_tick += 1;
        }
    });
    let mut decrypt_relay = use_signal(String::new);
    let mut me_refresh = use_signal(|| 0u64);

    let relay = use_hook(|| {
        NostrRelayManager::new(
            move |state| {
                connection_state.set(state);
                if matches!(
                    state,
                    RelayConnectionState::Error | RelayConnectionState::Disconnected
                ) {
                    loading.set(false);
                    if matches!(state, RelayConnectionState::Error) {
                        let keep_sign_in = panel_error
                            .peek()
                            .as_deref()
                            .is_some_and(|msg| msg.starts_with("The chat relay rejected sign-in"));
                        if !keep_sign_in {
                            panel_error.set(Some(
                                "Can't reach the chat relay. Check your connection and retry."
                                    .into(),
                            ));
                        }
                    }
                }
                if matches!(
                    state,
                    RelayConnectionState::Ready | RelayConnectionState::Connected
                ) {
                    panel_error.set(None);
                }
            },
            move |event| match event {
                NostrRelayEvent::ChatMessage(msg) => {
                    push_unique(&mut messages.write(), msg);
                }
                NostrRelayEvent::RawEvent { event, .. } => {
                    if event.kind != event_kinds::GIFT_WRAP {
                        return;
                    }
                    let relay_url = decrypt_relay.peek().clone();
                    spawn(async move {
                        match decrypt_gift_wrap(&event, &relay_url).await {
                            Ok(msg) => push_unique(&mut messages.write(), msg),
                            Err(e) => tracing::info!("Gift wrap decrypt skipped: {e}"),
                        }
                    });
                }
                NostrRelayEvent::Eose { .. } => {
                    loading.set(false);
                }
                NostrRelayEvent::AuthSuccess => {
                    connection_state.set(RelayConnectionState::Ready);
                    panel_error.set(None);
                }
                NostrRelayEvent::AuthRejected { message } => {
                    loading.set(false);
                    connection_state.set(RelayConnectionState::Error);
                    panel_error.set(Some(message));
                }
                NostrRelayEvent::Notice(msg) => {
                    tracing::info!("Relay notice: {}", msg);
                }
                NostrRelayEvent::SubscriptionRefused { message } => {
                    loading.set(false);
                    panel_error.set(Some(message));
                }
                NostrRelayEvent::EventPublished {
                    accepted: false,
                    message,
                    ..
                } => {
                    panel_error.set(Some(format!("Relay rejected the message: {message}")));
                }
                _ => {}
            },
        )
    });

    relay.set_auth_endpoint("/api/chat/auth-token");

    let relay_cleanup = relay.clone();
    use_drop(move || {
        relay_cleanup.disconnect();
    });

    let relay_state = use_relay_availability_state();
    let relay_connect = relay.clone();
    let channels_for_connect = channels.clone();
    use_effect(move || {
        let Some(dial) = relay_dial(relay_state.current(), &channels_for_connect) else {
            return;
        };
        let hint = match &dial {
            RelayDial::Absolute(url) => url.clone(),
            RelayDial::SameOrigin(path) => auth_relay_url(path),
        };
        decrypt_relay.set(hint);
        connect_relay(&relay_connect, &dial);
    });

    let me = use_resource(move || async move {
        let _ = me_refresh();
        ApiClient::web().get_me().await.ok()
    });

    let channels_for_keys = channels.clone();
    use_future(move || {
        let channels_for_keys = channels_for_keys.clone();
        async move {
            let has_officer = channels_for_keys
                .iter()
                .any(|c| c.group_type == GroupType::Officer && c.is_active);
            if !has_officer {
                return;
            }
            let Ok(me_now) = ApiClient::web().get_me().await else {
                return;
            };
            let has_key = me_now
                .member
                .as_ref()
                .and_then(|m| m.nostr_pubkey.as_ref())
                .is_some();
            if has_key {
                return;
            }
            let url = channels_for_keys
                .iter()
                .find(|c| c.group_type == GroupType::Officer)
                .map(|c| auth_relay_url(&c.relay_url))
                .unwrap_or_else(|| auth_relay_url(""));
            if provision_auth_token(&url).await.is_ok() {
                me_refresh += 1;
            }
        }
    });

    let relay_sub = relay.clone();
    let channels_for_sub = channels.clone();
    use_effect(move || {
        let conn = *connection_state.read();
        if !subscription_allowed(conn) {
            return;
        }
        let Some(group_id) = active_group.read().clone() else {
            return;
        };
        let Some(ch) = channel_by_group(&channels_for_sub, &group_id) else {
            return;
        };
        loading.set(true);
        if let Some(old) = current_sub_id.peek().as_ref() {
            relay_sub.unsubscribe(old);
        }
        if ch.group_type == GroupType::Public {
            let sub_id = relay_sub.subscribe_group(&group_id, Some(50));
            current_sub_id.set(Some(sub_id));
        } else {
            current_sub_id.set(None);
            loading.set(false);
        }
    });

    let relay_wraps = relay.clone();
    use_effect(move || {
        let conn = *connection_state.read();
        if !subscription_allowed(conn) {
            return;
        }
        let pubkey = me
            .read()
            .as_ref()
            .and_then(|o| o.as_ref())
            .and_then(|m: &MeResponse| m.member.as_ref())
            .and_then(|m| m.nostr_pubkey.clone());
        let Some(pubkey) = pubkey else {
            return;
        };
        if wrap_sub_id.peek().is_some() {
            return;
        }
        let filters = vec![NostrFilter::gift_wraps(&pubkey, Some(80))];
        let sub_id = relay_wraps.subscribe("wraps", filters);
        wrap_sub_id.set(Some(sub_id));
    });

    let conn = *connection_state.read();
    let is_loading = *loading.read();
    let is_expanded = *expanded.read() || embedded;
    let active = channels
        .iter()
        .find(|c| Some(c.group_id.as_str()) == active_group.read().as_deref())
        .cloned();
    let is_officer = active
        .as_ref()
        .is_some_and(|c| c.group_type == GroupType::Officer);
    let visible: Vec<ChatMessage> = messages
        .read()
        .iter()
        .filter(|m| Some(m.group_id.as_str()) == active_group.read().as_deref())
        .cloned()
        .collect();
    let me_data = me.read();
    let key_mode = key_mode_from_me(me_data.as_ref().and_then(|o| o.as_ref()));
    let err = panel_error.read().clone();
    let _hold_render = hold_tick();
    let public_send_wait = remaining_hold_secs(send_hold_until_ms(), js_sys::Date::now());
    let relay_retry = relay.clone();
    let active_channels: Vec<TeamChannel> =
        channels.iter().filter(|c| c.is_active).cloned().collect();

    let widget_class = if embedded {
        "chat-widget chat-widget--embedded"
    } else if is_expanded {
        "chat-widget"
    } else {
        "chat-widget chat-widget--collapsed"
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-components-chat-chat_widget-chat-widget-css", css: CHAT_WIDGET_CSS }
        div { class: "{widget_class}",

            div {
                class: "chat-widget__header",
                onclick: move |_| {
                    if !embedded {
                        expanded.toggle();
                    }
                },

                div { class: "chat-widget__header-left",
                    span { class: "chat-widget__title",
                        if let Some(ch) = active.as_ref() {
                            "{ch.group_type.label()} chat"
                        } else {
                            "Chat"
                        }
                    }
                    RelayStatus {
                        state: conn,
                        on_retry: move |_| {
                            panel_error.set(None);
                            relay_retry.retry();
                        },
                    }
                }
                if !embedded {
                    div { class: "chat-widget__header-right",
                        button {
                            class: "chat-widget__toggle",
                            onclick: move |evt| {
                                evt.stop_propagation();
                                expanded.toggle();
                            },
                            if is_expanded { "\u{25BC}" } else { "\u{25B2}" }
                        }
                    }
                }
            }

            div { class: "chat-widget__body",
                if !active_channels.is_empty() {
                    div { class: "chat-widget__channel-bar",
                        for channel in active_channels.iter() {
                            {
                                let is_on = active_group.read().as_deref() == Some(channel.group_id.as_str());
                                let group_id = channel.group_id.clone();
                                let label = channel.group_type.label();
                                rsx! {
                                    button {
                                        class: if is_on { "chat-widget__channel chat-widget__channel--active" } else { "chat-widget__channel" },
                                        onclick: move |_| {
                                            active_group.set(Some(group_id.clone()));
                                            panel_error.set(None);
                                        },
                                        "{label}"
                                    }
                                }
                            }
                        }
                    }
                }

                if !is_officer && public_send_wait > 0 {
                    div { class: "chat-widget__error", "{rate_limit_copy(public_send_wait)}" }
                } else if let Some(msg) = err {
                    div { class: "chat-widget__error", "{msg}" }
                }

                if active_channels.is_empty() {
                    div { class: "chat-widget__empty",
                        "No active chat channels for this team."
                    }
                } else if is_officer {
                    EncryptedChat {
                        messages: visible,
                        key_mode: key_mode,
                        loading: is_loading,
                        on_send: move |attempt: ComposeAttempt| {
                            if sending() {
                                attempt.fail();
                                return;
                            }
                            let Some(ch) = active.clone() else {
                                attempt.fail();
                                return;
                            };
                            sending.set(true);
                            panel_error.set(None);
                            spawn(async move {
                                let relay_url = auth_relay_url(&ch.relay_url);
                                match send_officer_message(&ch.group_id, &attempt.text, &relay_url)
                                    .await
                                {
                                    Ok(resp) => {
                                        let now = (js_sys::Date::now() / 1000.0) as u64;
                                        push_unique(
                                            &mut messages.write(),
                                            ChatMessage {
                                                id: format!("local-{}", now),
                                                pubkey: resp.sender_pubkey,
                                                display_name: None,
                                                avatar_url: None,
                                                content: attempt.text.clone(),
                                                created_at: now,
                                                group_id: ch.group_id,
                                                encrypted: true,
                                                reply_to: None,
                                            },
                                        );
                                        toast.show(Toast::success("Message sent"));
                                        attempt.succeed();
                                    }
                                    Err(e) => {
                                        panel_error.set(Some(e.clone()));
                                        toast.show(Toast::error(e));
                                        attempt.fail();
                                    }
                                }
                                sending.set(false);
                            });
                        },
                    }
                } else {
                    MessageList {
                        messages: visible,
                        loading: is_loading,
                    }
                    ComposeBox {
                        on_send: move |attempt: ComposeAttempt| {
                            if sending()
                                || remaining_hold_secs(send_hold_until_ms(), js_sys::Date::now())
                                    > 0
                            {
                                attempt.fail();
                                return;
                            }
                            let Some(ch) = active.clone() else {
                                attempt.fail();
                                return;
                            };
                            let relay_url = auth_relay_url(&ch.relay_url);
                            sending.set(true);
                            panel_error.set(None);
                            spawn(async move {
                                match send_public_message(&ch.group_id, &attempt.text, &relay_url)
                                    .await
                                {
                                    Ok(resp) => {
                                        push_unique(
                                            &mut messages.write(),
                                            ChatMessage {
                                                id: resp.event_id,
                                                pubkey: resp.sender_pubkey,
                                                display_name: None,
                                                avatar_url: None,
                                                content: attempt.text.clone(),
                                                created_at: resp.created_at,
                                                group_id: ch.group_id,
                                                encrypted: false,
                                                reply_to: None,
                                            },
                                        );
                                        attempt.succeed();
                                    }
                                    Err(err) => {
                                        if let Some(secs) = err.retry_after_secs {
                                            let until =
                                                js_sys::Date::now() + (secs as f64) * 1000.0;
                                            send_hold_until_ms.set(until);
                                            panel_error.set(None);
                                            attempt.rate_limited();
                                        } else {
                                            panel_error.set(Some(err.message.clone()));
                                            toast.show(Toast::error(err.message));
                                            attempt.fail();
                                        }
                                    }
                                }
                                sending.set(false);
                            });
                        },
                        send_locked: public_send_wait > 0,
                        placeholder: "Type a message...".to_string(),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(active: bool, relay_url: &str) -> TeamChannel {
        TeamChannel {
            id: "c".into(),
            team_id: "t".into(),
            group_id: "g".into(),
            group_type: GroupType::Public,
            relay_url: relay_url.into(),
            is_active: active,
            created_at: chrono::Utc::now(),
            synced_at: None,
        }
    }

    #[test]
    fn subscriptions_wait_out_the_auth_challenge() {
        assert!(subscription_allowed(RelayConnectionState::Connected));
        assert!(subscription_allowed(RelayConnectionState::Ready));
        assert!(!subscription_allowed(RelayConnectionState::Authenticating));
        assert!(!subscription_allowed(RelayConnectionState::Reconnecting(1)));
        assert!(!subscription_allowed(RelayConnectionState::Connecting));
        assert!(!subscription_allowed(RelayConnectionState::Error));
        assert!(!subscription_allowed(RelayConnectionState::Disconnected));
    }

    #[test]
    fn relay_off_does_not_dial_same_origin_relay() {
        let empty = channel(true, "");
        assert!(relay_dial(RelayAvailability::Off, std::slice::from_ref(&empty)).is_none());
        assert!(relay_dial(RelayAvailability::Unknown, std::slice::from_ref(&empty)).is_none());
        assert_eq!(
            relay_dial(RelayAvailability::On, std::slice::from_ref(&empty)),
            Some(RelayDial::SameOrigin("/relay".into()))
        );

        let http = channel(true, "http://relay.example");
        assert!(relay_dial(RelayAvailability::Off, std::slice::from_ref(&http)).is_none());
        assert_eq!(
            relay_dial(RelayAvailability::On, std::slice::from_ref(&http)),
            Some(RelayDial::SameOrigin("/relay".into()))
        );

        let absolute = channel(true, "wss://relay.example/path");
        assert_eq!(
            relay_dial(RelayAvailability::On, std::slice::from_ref(&absolute)),
            Some(RelayDial::Absolute("wss://relay.example/path".into()))
        );
        assert!(relay_dial(RelayAvailability::Off, std::slice::from_ref(&absolute)).is_none());

        let rooted = channel(true, "/custom");
        assert_eq!(
            relay_dial(RelayAvailability::On, std::slice::from_ref(&rooted)),
            Some(RelayDial::SameOrigin("/custom".into()))
        );

        let ws = channel(true, "ws://127.0.0.1:7777");
        assert_eq!(
            relay_dial(RelayAvailability::On, std::slice::from_ref(&ws)),
            Some(RelayDial::Absolute("ws://127.0.0.1:7777".into()))
        );

        let inactive = channel(false, "wss://relay.example");
        assert!(
            relay_dial(RelayAvailability::On, std::slice::from_ref(&inactive)).is_none(),
            "an inactive channel still does not open a socket"
        );
    }

    #[test]
    fn connect_effect_opens_a_socket_only_through_relay_dial() {
        let src = include_str!("chat_widget.rs");
        let start = src
            .find("let relay_connect = relay.clone()")
            .expect("connect effect");
        let rest = &src[start..];
        let end = rest.find("let me = use_resource").expect("me resource");
        let effect = &rest[..end];
        let dial_at = effect.find("relay_dial(").expect("dial");
        let connect_at = effect.find("connect_relay(").expect("connect");
        assert!(
            dial_at < connect_at,
            "the socket opens only after relay_dial returns a target: {effect}"
        );
        let prod = src.split("mod tests").next().expect("production source");
        assert_eq!(
            prod.matches("connect_relay(").count(),
            2,
            "definition plus the one effect call"
        );
        assert_eq!(prod.matches("relay.connect(").count(), 1);
        assert_eq!(prod.matches("connect_same_origin(").count(), 1);
    }

    #[test]
    fn public_send_posts_to_the_session_endpoint() {
        let src = include_str!("chat_widget.rs");
        let prod = src.split("mod tests").next().expect("production source");
        assert!(prod.contains("/api/chat/send-public"));
        assert!(prod.contains("send_public_message"));
        assert!(!prod.contains("nip07_sign_kind9"));
        assert!(!prod.contains("has_nip07"));
        assert!(prod.contains("send_officer_message"));
        assert!(prod.contains("/api/chat/send-encrypted"));
    }

    #[test]
    fn public_send_errors_use_plain_language() {
        let too_long = public_send_failure(
            400,
            r#"{"error":"Message is too long (max 2000 characters)"}"#,
            None,
        );
        assert_eq!(too_long.message, "Message is too long.");
        assert_eq!(too_long.retry_after_secs, None);

        let empty = public_send_failure(
            400,
            r#"{"error":"Message content must not be empty"}"#,
            None,
        );
        assert_eq!(empty.message, "Write a message before sending.");

        let blank_group =
            public_send_failure(400, r#"{"error":"group_id must not be empty"}"#, None);
        assert_eq!(blank_group.message, "That message couldn't be sent.");

        let external = public_send_failure(
            400,
            r#"{"error":"External key users must sign client-side (NIP-07)"}"#,
            None,
        );
        assert_eq!(external.message, "That message couldn't be sent.");

        let missing_keys = public_send_failure(
            400,
            r#"{"error":"No Nostr keys provisioned. Call /api/chat/auth-token first."}"#,
            None,
        );
        assert_eq!(missing_keys.message, "That message couldn't be sent.");
        assert!(needs_key_provision(
            r#"{"error":"No Nostr keys provisioned. Call /api/chat/auth-token first."}"#
        ));
        assert!(!needs_key_provision(
            r#"{"error":"External key users must sign client-side (NIP-07)"}"#
        ));

        assert_eq!(
            public_send_failure(401, r#"{"error":"Unauthorized"}"#, None).message,
            "Sign in to send a message."
        );
        assert_eq!(
            public_send_failure(403, r#"{"error":"Not a member of this team"}"#, None).message,
            "You are not allowed in this channel."
        );
        assert_eq!(
            public_send_failure(404, r#"{"error":"Channel 'alpha' not found"}"#, None).message,
            "Channel not found."
        );
        assert_eq!(
            public_send_failure(
                502,
                r#"{"error":"Relay rejected or failed to accept the message"}"#,
                None
            )
            .message,
            "Couldn't reach the chat server."
        );
        assert_eq!(
            public_send_failure(
                503,
                r#"{"error":"Cannot connect to relay for publishing"}"#,
                None
            )
            .message,
            "Couldn't reach the chat server."
        );
        assert_eq!(
            public_send_failure(500, r#"{"error":"Internal error"}"#, None).message,
            "Something went wrong sending that message."
        );

        let limited =
            public_send_failure(429, r#"{"error":"rate_limited","retry_after":12}"#, Some(3));
        assert_eq!(limited.retry_after_secs, Some(12));
        assert_eq!(
            limited.message,
            "Too many messages. Try again in 12 seconds."
        );

        let one = public_send_failure(429, r#"{"error":"rate_limited","retry_after":1}"#, None);
        assert_eq!(one.retry_after_secs, Some(1));
        assert_eq!(one.message, "Too many messages. Try again in 1 second.");

        let from_header = public_send_failure(429, "nope", Some(4));
        assert_eq!(from_header.retry_after_secs, Some(4));
        assert_eq!(
            from_header.message,
            "Too many messages. Try again in 4 seconds."
        );

        let numeric_string =
            public_send_failure(429, r#"{"error":"rate_limited","retry_after":"8"}"#, None);
        assert_eq!(numeric_string.retry_after_secs, Some(8));

        let later =
            public_send_failure(429, r#"{"error":"rate_limited","retry_after":3601}"#, None);
        assert_eq!(later.message, "Too many messages. Try again later.");
        assert_eq!(later.retry_after_secs, None);

        let fraction =
            public_send_failure(429, r#"{"error":"rate_limited","retry_after":1.5}"#, None);
        assert_eq!(fraction.retry_after_secs, None);
        assert_eq!(fraction.message, "Too many messages. Try again later.");
    }

    #[test]
    fn rate_limit_hold_counts_down_to_zero() {
        assert_eq!(remaining_hold_secs(1_000_000.0, 995_000.0), 5);
        assert_eq!(remaining_hold_secs(1_000_000.0, 999_000.0), 1);
        assert_eq!(remaining_hold_secs(1_000_000.0, 999_001.0), 1);
        assert_eq!(remaining_hold_secs(1_000_000.0, 1_000_000.0), 0);
        assert_eq!(remaining_hold_secs(1_000_000.0, 1_000_001.0), 0);
        assert_eq!(remaining_hold_secs(0.0, 0.0), 0);
        assert_eq!(remaining_hold_secs(f64::NAN, 0.0), 0);
        let now = 1_700_000_000_000.0;
        let until = now + 12_000.0;
        assert_eq!(remaining_hold_secs(until, now), 12);
        assert_eq!(remaining_hold_secs(until, now + 11_001.0), 1);
        assert_eq!(remaining_hold_secs(until, now + 12_000.0), 0);
    }

    #[test]
    fn push_unique_drops_a_relay_echo_of_the_same_event() {
        let mut messages = Vec::new();
        let sent = ChatMessage {
            id: "abc".into(),
            pubkey: "pk".into(),
            display_name: None,
            avatar_url: None,
            content: "hello team".into(),
            created_at: 10,
            group_id: "g".into(),
            encrypted: false,
            reply_to: None,
        };
        push_unique(&mut messages, sent.clone());
        push_unique(&mut messages, sent);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "hello team");
    }
}
