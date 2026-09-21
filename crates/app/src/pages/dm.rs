use dioxus::prelude::*;

use scuffed_api_client::ApiClient;
use scuffed_types::MeResponse;

use crate::components::dm::{
    ConversationList, ConversationSummary, DmComposeModal, MessageThread, SyncResponse,
};
use crate::components::{Toast, use_toast};
use crate::routes::Route;
use crate::state::auth::use_auth;

const PAGE_CSS: &str = r#"
.dm-page {
    padding: 2rem;
    max-width: 1100px;
    margin: 0 auto;
    box-sizing: border-box;
}
.dm-page-title {
    font-family: var(--font-head);
    font-size: 2.25rem;
    color: var(--text);
    letter-spacing: 3px;
    margin: 0 0 1.25rem;
}
.dm-page-grid {
    display: grid;
    grid-template-columns: 320px 1fr;
    gap: 1rem;
    height: calc(100vh - 220px);
    min-height: 480px;
}
.dm-page-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    color: var(--text-3);
    font-size: 0.95rem;
    text-align: center;
    padding: 2rem;
}
.dm-loading {
    color: var(--text-3);
    text-align: center;
    padding: 3rem 0;
    font-size: 0.9rem;
}
.dm-error {
    background: color-mix(in srgb, var(--danger) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    border-radius: 8px;
    padding: 1rem 1.25rem;
    color: var(--danger);
    font-size: 0.85rem;
    margin-bottom: 1rem;
}
.dm-error a {
    color: var(--danger);
    font-weight: 600;
    text-decoration: underline;
}
.dm-login-needed {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 2.5rem 2rem;
    text-align: center;
    color: var(--text-2);
}
@media (max-width: 720px) {
    .dm-page-grid {
        grid-template-columns: 1fr;
        height: auto;
    }
}
"#;

#[derive(Clone, Copy, PartialEq)]
enum LoadState {
    Loading,
    Ready,
    NeedsServerManaged,
    OtherError,
}

#[component]
pub fn DmInbox() -> Element {
    rsx! { DmPageInner { selected_peer: None } }
}

#[component]
pub fn DmThread(peer_pubkey: String) -> Element {
    rsx! { DmPageInner { selected_peer: Some(peer_pubkey) } }
}

#[component]
fn DmPageInner(selected_peer: Option<String>) -> Element {
    // Hooks must run unconditionally. Auth boots as loading → logged-out, then
    // `/api/auth/me` lands; an early return before these hooks changes the
    // hook count and panics (same pattern as AdminLayout).
    let auth = use_auth();
    let mut toast = use_toast();
    let mut refresh = use_signal(|| 0u64);
    let mut syncing = use_signal(|| false);
    let mut load_state = use_signal(|| LoadState::Loading);
    let mut conversations = use_signal::<Vec<ConversationSummary>>(Vec::new);
    let mut compose_open = use_signal(|| false);
    let navigator = use_navigator();

    let me = use_resource(move || async move {
        // Read auth so this resource re-runs when the session resolves.
        if !auth().is_logged_in() {
            return None;
        }
        ApiClient::web().get_me().await.ok()
    });

    let _load_conversations = use_resource(move || async move {
        let _ = refresh();
        if !auth().is_logged_in() {
            load_state.set(LoadState::Loading);
            return;
        }
        load_state.set(LoadState::Loading);

        // Fire sync on mount; ignore failure (will surface via the conversations call).
        let _ = ApiClient::web()
            .post_json::<_, SyncResponse>("/api/nostr/dm/sync", &serde_json::json!({}))
            .await;

        match ApiClient::web()
            .fetch::<Vec<ConversationSummary>>("/api/nostr/dm/conversations")
            .await
        {
            Ok(list) => {
                conversations.set(list);
                load_state.set(LoadState::Ready);
            }
            Err(scuffed_api_client::ClientError::Http { status, body }) => {
                let needs_managed = status == 400
                    || status == 403
                    || status == 409
                    || status == 412
                    || body.contains("server_managed");
                load_state.set(if needs_managed {
                    LoadState::NeedsServerManaged
                } else {
                    LoadState::OtherError
                });
            }
            Err(_) => {
                load_state.set(LoadState::OtherError);
            }
        }
    });

    if auth().loading {
        return rsx! {
            style { {PAGE_CSS} }
            main { class: "dm-page",
                h1 { class: "dm-page-title", "Direct Messages" }
                p { class: "dm-loading", "Checking session…" }
            }
        };
    }

    if !auth().is_logged_in() {
        return rsx! {
            style { {PAGE_CSS} }
            main { class: "dm-page",
                h1 { class: "dm-page-title", "Direct Messages" }
                div { class: "dm-login-needed",
                    "You must be signed in to view direct messages."
                }
            }
        };
    }

    let on_refresh = move |_| {
        if syncing() {
            return;
        }
        syncing.set(true);
        spawn(async move {
            match ApiClient::web()
                .post_json::<_, SyncResponse>("/api/nostr/dm/sync", &serde_json::json!({}))
                .await
            {
                Ok(resp) => {
                    if resp.fetched > 0 {
                        toast.show(Toast::success(format!(
                            "Synced {} new message(s)",
                            resp.stored
                        )));
                    }
                }
                Err(e) => toast.show(Toast::error(format!("Sync failed: {e}"))),
            }
            syncing.set(false);
            refresh += 1;
        });
    };

    let on_compose = move |_| {
        compose_open.set(true);
    };

    let on_compose_close = move |_| {
        compose_open.set(false);
    };

    let on_compose_sent = move |(recipient_hex, _msg): (String, _)| {
        compose_open.set(false);
        // Trigger a sync+refresh so the new conversation row appears with
        // accurate unread/preview state, then route to the thread. The
        // MessageThread component will fetch its own page on mount and the
        // server already stored the sender's copy synchronously inside
        // /api/nostr/dm/send.
        refresh += 1;
        navigator.push(Route::DmThread {
            peer_pubkey: recipient_hex,
        });
    };

    let me_data = me.read();
    let self_pubkey = me_data
        .as_ref()
        .and_then(|o| o.as_ref())
        .and_then(|me: &MeResponse| me.member.as_ref())
        .and_then(|m| m.nostr_pubkey.clone());

    let convs = conversations();
    let selected_summary = selected_peer
        .as_ref()
        .and_then(|pk| convs.iter().find(|c| c.peer_pubkey == *pk).cloned());

    rsx! {
        style { {PAGE_CSS} }
        main { class: "dm-page",
            h1 { class: "dm-page-title", "Direct Messages" }

            if matches!(load_state(), LoadState::NeedsServerManaged) {
                div { class: "dm-error",
                    "Direct messages require a server-managed Nostr identity. "
                    Link { to: Route::IdentitySettings {}, "Visit identity settings" }
                    " to enable it."
                }
            }
            if matches!(load_state(), LoadState::OtherError) {
                div { class: "dm-error",
                    "Could not load conversations. Try Refresh, or check your connection."
                }
            }

            div { class: "dm-page-grid",
                ConversationList {
                    conversations: convs.clone(),
                    selected_peer: selected_peer.clone(),
                    refreshing: syncing(),
                    on_refresh: on_refresh,
                    on_compose: on_compose,
                }
                {match selected_peer.as_ref() {
                    None => {
                        let msg = if matches!(load_state(), LoadState::Loading) {
                            "Loading…"
                        } else if convs.is_empty() {
                            "Select a conversation to start reading."
                        } else {
                            "Select a conversation from the left."
                        };
                        rsx! {
                            div { class: "dm-page-empty", "{msg}" }
                        }
                    }
                    Some(peer) => {
                        let display_name = selected_summary
                            .as_ref()
                            .and_then(|s| s.peer_display_name.clone());
                        rsx! {
                            MessageThread {
                                key: "{peer}",
                                peer_pubkey: peer.clone(),
                                peer_display_name: display_name,
                                self_pubkey: self_pubkey.clone(),
                            }
                        }
                    }
                }}
            }

            DmComposeModal {
                open: compose_open(),
                self_pubkey: self_pubkey.clone(),
                on_close: on_compose_close,
                on_sent: on_compose_sent,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// Source-order guard: auth/session flips must not change the hook count.
    /// The old `if !auth().is_logged_in() { return }` sat *before* `use_signal` /
    /// `use_resource` / `use_navigator` and panicked when `/api/auth/me` landed.
    #[test]
    fn dm_page_inner_registers_hooks_before_any_return() {
        let src = include_str!("dm.rs");
        let start = src.find("fn DmPageInner").expect("DmPageInner component");
        let body = &src[start..];
        let first_return = body
            .find("return rsx!")
            .expect("expected a post-hooks return rsx!");
        for hook in [
            "use_auth()",
            "use_toast()",
            "use_signal",
            "use_navigator()",
            "use_resource",
        ] {
            let pos = body
                .find(hook)
                .unwrap_or_else(|| panic!("{hook} should appear in DmPageInner"));
            assert!(
                pos < first_return,
                "{hook} must be registered before the first `return rsx!` (was {pos} >= {first_return})"
            );
        }
    }
}
