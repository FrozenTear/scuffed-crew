//! Main expandable chat widget — sidebar/panel layout.
//!
//! Wires F-API-003/004: discovered `TeamChannel` group_ids, NIP-29 kind 9 on
//! public channels, and send-encrypted / decrypt on officer channels.

use dioxus::prelude::*;
use scuffed_api_client::{ApiClient, ClientError};
use scuffed_types::nostr::{ChatMessage, NostrEvent, NostrFilter, event_kinds};
use scuffed_types::{
    AuthTokenRequest, AuthTokenResponse, DecryptMessageRequest, DecryptMessageResponse, GroupType,
    MeResponse, SendEncryptedRequest, SendEncryptedResponse, TeamChannel, chat_api_error_copy,
};

use crate::components::chat::encrypted_chat::{EncryptedChat, KeyMode};
use crate::components::{Toast, use_toast};
use crate::state::nostr::{NostrRelayEvent, NostrRelayManager, RelayConnectionState};

use super::compose_box::ComposeBox;
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

.chat-widget__hint {
    margin: 8px 12px 0;
    padding: 8px 10px;
    border-radius: 8px;
    background: var(--surface-2);
    color: var(--text-2);
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

fn connect_relay(relay: &NostrRelayManager, channels: &[TeamChannel]) {
    let url = channels
        .iter()
        .find(|c| c.is_active)
        .map(|c| c.relay_url.as_str())
        .unwrap_or("");
    if url.starts_with("ws://") || url.starts_with("wss://") {
        relay.connect(url);
    } else if url.starts_with('/') {
        relay.connect_same_origin(url);
    } else {
        relay.connect_same_origin("/relay");
    }
}

fn has_nip07() -> bool {
    web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &wasm_bindgen::JsValue::from_str("nostr")).ok())
        .map(|v| !v.is_undefined() && !v.is_null())
        .unwrap_or(false)
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
            ClientError::Http { status, body } => chat_api_error_copy(status, &body),
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
        Err(ClientError::Http { status: 400, body }) => {
            if body.contains("External key") {
                return Err(chat_api_error_copy(400, &body));
            }
            provision_auth_token(relay_url).await?;
            match client
                .post_json::<_, SendEncryptedResponse>("/api/chat/send-encrypted", &req)
                .await
            {
                Ok(r) => Ok(r),
                Err(ClientError::Http { status, body }) => Err(chat_api_error_copy(status, &body)),
                Err(e) => Err(e.to_string()),
            }
        }
        Err(ClientError::Http { status, body }) => Err(chat_api_error_copy(status, &body)),
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
        Err(ClientError::Http { status: 400, body }) => {
            if body.contains("External key") {
                return Err(chat_api_error_copy(400, &body));
            }
            let _ = provision_auth_token(relay_url).await;
            client
                .post_json::<_, DecryptMessageResponse>("/api/chat/decrypt", &req)
                .await
                .map_err(|e| match e {
                    ClientError::Http { status, body } => chat_api_error_copy(status, &body),
                    other => other.to_string(),
                })?
        }
        Err(ClientError::Http { status, body }) => return Err(chat_api_error_copy(status, &body)),
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

async fn nip07_sign_kind9(group_id: &str, content: &str) -> Result<NostrEvent, String> {
    use wasm_bindgen::{JsCast, JsValue};

    let window = web_sys::window().ok_or("No window")?;
    let nostr = js_sys::Reflect::get(&window, &JsValue::from_str("nostr"))
        .map_err(|_| "NIP-07 extension not found")?;
    if nostr.is_undefined() || nostr.is_null() {
        return Err("NIP-07 extension not found".into());
    }

    let created_at = (js_sys::Date::now() / 1000.0) as u64;
    let event_obj = js_sys::Object::new();
    js_sys::Reflect::set(
        &event_obj,
        &JsValue::from_str("kind"),
        &JsValue::from_f64(9.0),
    )
    .map_err(|_| "Failed to set kind")?;
    js_sys::Reflect::set(
        &event_obj,
        &JsValue::from_str("content"),
        &JsValue::from_str(content),
    )
    .map_err(|_| "Failed to set content")?;
    js_sys::Reflect::set(
        &event_obj,
        &JsValue::from_str("created_at"),
        &JsValue::from_f64(created_at as f64),
    )
    .map_err(|_| "Failed to set created_at")?;

    let tags = js_sys::Array::new();
    let htag = js_sys::Array::new();
    htag.push(&JsValue::from_str("h"));
    htag.push(&JsValue::from_str(group_id));
    tags.push(&htag);
    js_sys::Reflect::set(&event_obj, &JsValue::from_str("tags"), &tags)
        .map_err(|_| "Failed to set tags")?;

    let sign = js_sys::Reflect::get(&nostr, &JsValue::from_str("signEvent"))
        .map_err(|_| "signEvent missing")?;
    let sign_fn: js_sys::Function = sign.dyn_into().map_err(|_| "signEvent is not a function")?;
    let promise = sign_fn
        .call1(&nostr, &event_obj)
        .map_err(|e| format!("{e:?}"))?;
    let promise: js_sys::Promise = promise
        .dyn_into()
        .map_err(|_| "signEvent did not return a promise")?;
    let signed = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|e| format!("NIP-07 signEvent failed: {e:?}"))?;
    serde_wasm_bindgen::from_value(signed).map_err(|e| e.to_string())
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
                        panel_error.set(Some(
                            "Can't reach the chat relay. Check your connection and retry.".into(),
                        ));
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
                NostrRelayEvent::Notice(msg) => {
                    tracing::info!("Relay notice: {}", msg);
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

    let relay_connect = relay.clone();
    let channels_for_connect = channels.clone();
    use_effect(move || {
        if channels_for_connect.iter().any(|c| c.is_active) {
            let hint = channels_for_connect
                .iter()
                .find(|c| c.is_active)
                .map(|c| auth_relay_url(&c.relay_url))
                .unwrap_or_else(|| auth_relay_url(""));
            decrypt_relay.set(hint);
            connect_relay(&relay_connect, &channels_for_connect);
        }
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
        if !conn.is_connected() {
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
        if !conn.is_connected() {
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
        style { {CHAT_WIDGET_CSS} }
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

                if let Some(msg) = err {
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
                        on_send: move |text: String| {
                            if sending() {
                                return;
                            }
                            let Some(ch) = active.clone() else { return };
                            sending.set(true);
                            panel_error.set(None);
                            spawn(async move {
                                let relay_url = auth_relay_url(&ch.relay_url);
                                match send_officer_message(&ch.group_id, &text, &relay_url).await {
                                    Ok(resp) => {
                                        let now = (js_sys::Date::now() / 1000.0) as u64;
                                        push_unique(
                                            &mut messages.write(),
                                            ChatMessage {
                                                id: format!("local-{}", now),
                                                pubkey: resp.sender_pubkey,
                                                display_name: None,
                                                avatar_url: None,
                                                content: text,
                                                created_at: now,
                                                group_id: ch.group_id,
                                                encrypted: true,
                                                reply_to: None,
                                            },
                                        );
                                        toast.show(Toast::success("Message sent"));
                                    }
                                    Err(e) => {
                                        panel_error.set(Some(e.clone()));
                                        toast.show(Toast::error(e));
                                    }
                                }
                                sending.set(false);
                            });
                        },
                    }
                } else {
                    if !has_nip07() {
                        div { class: "chat-widget__hint",
                            "Public messages are NIP-29 kind 9 on the discovered group. \
                            Sending needs a NIP-07 extension — server-managed keys can listen here \
                            and send on officer channels via the API."
                        }
                    }
                    MessageList {
                        messages: visible,
                        loading: is_loading,
                    }
                    ComposeBox {
                        on_send: move |text: String| {
                            if sending() {
                                return;
                            }
                            let Some(ch) = active.clone() else { return };
                            if !has_nip07() {
                                let msg = "Public channel send needs a NIP-07 extension.".to_string();
                                panel_error.set(Some(msg.clone()));
                                toast.show(Toast::error(msg));
                                return;
                            }
                            let relay_pub = relay.clone();
                            sending.set(true);
                            panel_error.set(None);
                            spawn(async move {
                                match nip07_sign_kind9(&ch.group_id, &text).await {
                                    Ok(event) => {
                                        let pubkey = event.pubkey.clone();
                                        let id = event.id.clone();
                                        relay_pub.publish_event(event);
                                        let now = (js_sys::Date::now() / 1000.0) as u64;
                                        push_unique(
                                            &mut messages.write(),
                                            ChatMessage {
                                                id,
                                                pubkey,
                                                display_name: None,
                                                avatar_url: None,
                                                content: text,
                                                created_at: now,
                                                group_id: ch.group_id,
                                                encrypted: false,
                                                reply_to: None,
                                            },
                                        );
                                    }
                                    Err(e) => {
                                        panel_error.set(Some(e.clone()));
                                        toast.show(Toast::error(e));
                                    }
                                }
                                sending.set(false);
                            });
                        },
                        disabled: !conn.is_ready() || sending(),
                        placeholder: if conn.is_ready() {
                            "Type a message...".to_string()
                        } else {
                            "Connecting to relay...".to_string()
                        },
                    }
                }
            }
        }
    }
}
