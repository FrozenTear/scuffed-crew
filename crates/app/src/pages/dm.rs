use dioxus::prelude::*;

use scuffed_api_client::ApiClient;
use scuffed_types::MeResponse;

use crate::components::dm::{
    ConversationList, ConversationListState, ConversationSummary, DmComposeModal, DmLoadFailure,
    DmMember, DmMemberPage, MessageThread, SyncResponse, classify_dm_client_error, peer_label,
};
use crate::components::member_gate::Wall;
use crate::components::ui::{PageHeader, PageShell, PageWidth, SkeletonBars, btn_class};
use crate::components::{Toast, use_toast};
use crate::head::use_page_title;
use crate::pages::chat_gate::ChatGateNotice;
use crate::routes::Route;
use crate::state::auth::use_auth;
use crate::state::relay::dm_may_sync;
use crate::state::use_relay_availability_state;

/// The full-height tool of the `d1-dm` frames. The shell hides the footer on
/// this route and sets `--shell-chrome-height` (the top bar, plus the tab bar
/// on phone), so the page takes exactly the rest of the viewport: the inbox
/// column and the thread side by side on desktop, one at a time on phone
/// (the list on `/dm`, the thread with Back on `/dm/:peer`), the composer
/// pinned at the bottom, just above the tab bar on phone.
const PAGE_CSS: &str = r#"
.dm-page {
    box-sizing: border-box;
    width: 100%;
    max-width: var(--content-max-full);
    height: calc(100vh - var(--shell-chrome-height));
    height: calc(100dvh - var(--shell-chrome-height));
    display: flex;
    flex-direction: column;
    background: var(--bg);
}
.dm-layout {
    flex: 1 1 auto;
    min-height: 0;
    display: grid;
    grid-template-columns: calc(var(--space-8) * 10) minmax(0, 1fr);
}
.dm-inbox,
.dm-thread-pane {
    position: relative;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--surface);
}
.dm-inbox { border-right: 1px solid var(--border); }
.dm-pane-head {
    box-sizing: border-box;
    flex: 0 0 auto;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    min-height: var(--hit-target-min);
    padding: var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--border);
}
.dm-pane-title {
    margin: 0;
    min-width: 0;
    font-family: var(--font-head);
    font-size: var(--text-lg);
    font-weight: var(--font-head-weight);
    letter-spacing: var(--font-head-tracking);
    line-height: var(--leading-ui);
    color: var(--text);
}
.dm-avatar {
    box-sizing: border-box;
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--control-size-sm);
    height: var(--control-size-sm);
    border-radius: var(--radius-pill);
    border: 1px solid var(--border-strong);
    background: var(--surface);
    color: var(--text);
    font-family: var(--font-head);
    font-weight: var(--font-head-weight);
    font-size: var(--text-xs);
}
.dm-thread-empty {
    flex: 1 1 auto;
    display: flex;
    align-items: center;
    justify-content: center;
    margin: 0;
    padding: var(--space-section);
    background: var(--bg);
    color: var(--text-3);
    font-size: var(--text-md);
    line-height: var(--leading-body);
    text-align: center;
}
.dm-wait-status {
    margin: 0;
    color: var(--text-3);
    font-size: var(--text-md);
}
@media (max-width: 820px) {
    .dm-layout { grid-template-columns: minmax(0, 1fr); }
    .dm-inbox { border-right: 0; }
    .dm-page.is-thread .dm-inbox { display: none; }
    .dm-page.is-inbox .dm-thread-pane { display: none; }
    .dm-page .dm-thread-head .dm-thread-back { display: inline-flex; }
    .dm-avatar {
        width: var(--hit-target-min);
        height: var(--hit-target-min);
        font-size: var(--text-sm);
    }
}
"#;

const PAGE_CSS_ID: &str = "scuffed-css-pages-dm-page-css";

/// Pages of `/api/members` read for names. The server caps a page at 100.
const MEMBER_PAGES_MAX: usize = 10;

#[derive(Clone, PartialEq)]
enum LoadState {
    Loading,
    Ready,
    Error(DmLoadFailure),
}

fn list_state(state: &LoadState) -> ConversationListState {
    match state {
        LoadState::Loading => ConversationListState::Loading,
        LoadState::Ready => ConversationListState::Ready,
        LoadState::Error(_) => ConversationListState::Failed,
    }
}

/// Thread-pane copy when no conversation is open (desktop only; phone shows
/// the list). Loading, failure and an empty inbox are said once, in the
/// inbox column (its empty card already says how to start one).
fn inbox_pane_message(state: &LoadState, conversation_count: usize) -> Option<&'static str> {
    match state {
        LoadState::Loading | LoadState::Error(_) => None,
        LoadState::Ready if conversation_count == 0 => None,
        LoadState::Ready => Some("Pick a conversation to read it."),
    }
}

/// The Refresh toast, or none when nothing new arrived.
fn synced_toast(stored: u64) -> Option<String> {
    match stored {
        0 => None,
        1 => Some("1 new message.".to_string()),
        n => Some(format!("{n} new messages.")),
    }
}

/// Said under the inbox head when the background sync could not reach the
/// relay: the list is what the server already has.
const SYNC_FAILED: &str = "Could not reach the relay. Showing saved messages.";

/// The conversation to show on `/dm` without a click: the only one, on a
/// wide screen. On phone the list comes first, so nothing opens (and nothing
/// is marked read unseen).
fn auto_open_peer(conversations: &[ConversationSummary], wide: bool) -> Option<String> {
    match conversations {
        [only] if wide => Some(only.peer_pubkey.clone()),
        _ => None,
    }
}

/// True when the inbox and the thread sit side by side (above the shell's
/// 820px phone break).
fn wide_viewport() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.match_media("(min-width: 821px)").ok().flatten())
            .is_some_and(|mql| mql.matches())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
}

/// Crew members with their Nostr keys, to show peers by name. A failed read
/// keeps what it has: names fall back to short npub labels.
async fn fetch_members() -> Vec<DmMember> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MEMBER_PAGES_MAX {
        let url = match &cursor {
            Some(c) => format!(
                "/api/members?limit=100&cursor={}",
                crate::util::encode_query(c)
            ),
            None => "/api/members?limit=100".to_string(),
        };
        match ApiClient::web().fetch::<DmMemberPage>(&url).await {
            Ok(page) => {
                out.extend(page.data);
                match page.next_cursor {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
            Err(err) => {
                crate::util::log_client_error("DM member names", &err);
                break;
            }
        }
    }
    out
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
    let relay = use_relay_availability_state();
    let availability = relay.current();
    let auth = use_auth();
    let mut toast = use_toast();
    let mut refresh = use_signal(|| 0u64);
    let mut syncing = use_signal(|| false);
    let mut load_state = use_signal(|| LoadState::Loading);
    let mut conversations = use_signal::<Vec<ConversationSummary>>(Vec::new);
    let mut compose_open = use_signal(|| false);
    let mut sync_notice = use_signal(|| None::<String>);
    let navigator = use_navigator();

    let me = use_resource(move || async move {
        // Read auth so this resource re-runs when the session resolves.
        if !dm_may_sync(relay.current(), auth().is_logged_in()) {
            return None;
        }
        ApiClient::web().get_me().await.ok()
    });

    let members = use_resource(move || async move {
        if !dm_may_sync(relay.current(), auth().is_logged_in()) {
            return Vec::new();
        }
        fetch_members().await
    });

    let _load_conversations = use_resource(move || async move {
        let _ = refresh();
        // Health unknown or relay off: do not sync or list. A sync against a
        // missing relay is the 503 the page used to show.
        if !dm_may_sync(relay.current(), auth().is_logged_in()) {
            if !auth().is_logged_in() {
                load_state.set(LoadState::Loading);
            }
            return;
        }
        load_state.set(LoadState::Loading);

        // Sync hits the relay and can hang. Do not await it before the list;
        // a stuck sync used to leave the inbox on the empty/in-flight state forever.
        // When it lands: new messages reload the list (without another sync,
        // so this never loops), and a failure is said, not hidden behind
        // "No conversations yet".
        spawn(async move {
            match ApiClient::web()
                .post_json::<_, SyncResponse>("/api/nostr/dm/sync", &serde_json::json!({}))
                .await
            {
                Ok(resp) => {
                    sync_notice.set(None);
                    if resp.stored > 0
                        && let Ok(list) = ApiClient::web()
                            .fetch::<Vec<ConversationSummary>>("/api/nostr/dm/conversations")
                            .await
                    {
                        conversations.set(list);
                        load_state.set(LoadState::Ready);
                    }
                }
                Err(err) => {
                    crate::util::log_client_error("DM sync", &err);
                    sync_notice.set(Some(SYNC_FAILED.to_string()));
                }
            }
        });

        match ApiClient::web()
            .fetch::<Vec<ConversationSummary>>("/api/nostr/dm/conversations")
            .await
        {
            Ok(list) => {
                conversations.set(list);
                load_state.set(LoadState::Ready);
            }
            Err(err) => {
                load_state.set(LoadState::Error(classify_dm_client_error(&err)));
            }
        }
    });

    let member_list: Vec<DmMember> = members.read().as_ref().cloned().unwrap_or_default();
    let convs = conversations();
    let label_for = |pk: &str| {
        let server_name = convs
            .iter()
            .find(|c| c.peer_pubkey == pk)
            .and_then(|c| c.peer_display_name.clone());
        peer_label(pk, server_name.as_deref(), &member_list)
    };
    let route_peer_label = selected_peer.as_deref().map(&label_for);
    use_page_title(
        route_peer_label
            .clone()
            .map(|name| format!("Messages with {name}")),
    );

    if !availability.allows_connection() {
        return rsx! {
            ChatGateNotice {
                availability,
                on_retry: move |_| relay.retry(),
            }
        };
    }

    if auth().loading {
        return rsx! {
            PageShell { width: PageWidth::Narrow, class: "dm-wait",
                PageHeader { title: "Messages" }
                p { class: "dm-wait-status", role: "status", "Checking your session..." }
                SkeletonBars { rows: 3 }
            }
            crate::styles::StyleOnce { id: PAGE_CSS_ID, css: PAGE_CSS }
        };
    }

    if !auth().is_logged_in() {
        return rsx! {
            PageShell { width: PageWidth::Narrow, class: "dm-signed-out",
                Wall {
                    page_title: true,
                    title: "Log in to read your messages",
                    lead: "Direct messages are private to crew members. Log in to see your conversations and send new ones.",
                    actions: rsx! {
                        Link { to: Route::Login {}, class: btn_class(true), "Log in" }
                        Link { to: Route::Home {}, class: btn_class(false), "Return home" }
                    },
                }
            }
        };
    }

    let on_refresh = move |_| {
        if !dm_may_sync(relay.current(), auth().is_logged_in()) {
            return;
        }
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
                    sync_notice.set(None);
                    if let Some(text) = synced_toast(resp.stored) {
                        toast.show(Toast::success(text));
                    }
                }
                Err(e) => {
                    sync_notice.set(Some(SYNC_FAILED.to_string()));
                    toast.show(Toast::error(format!("Sync failed: {e}")));
                }
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

    // The open thread is read: clear its row's count here too, so it stays
    // clear after the thread is closed, without a refetch.
    let on_read = move |peer: String| {
        conversations.with_mut(|list| {
            for c in list.iter_mut().filter(|c| c.peer_pubkey == peer) {
                c.unread_count = 0;
            }
        });
    };

    let me_data = me.read();
    let me_member = me_data
        .as_ref()
        .and_then(|o| o.as_ref())
        .and_then(|me: &MeResponse| me.member.as_ref());
    let self_pubkey = me_member.and_then(|m| m.nostr_pubkey.clone());
    let self_label = me_member.map(|m| m.display_name.clone());

    let state = load_state();
    let failure = match &state {
        LoadState::Error(failure) => Some(failure.clone()),
        _ => None,
    };
    let on_thread_route = selected_peer.is_some();
    let open_peer = selected_peer.clone().or_else(|| {
        if state == LoadState::Ready {
            auto_open_peer(&convs, wide_viewport())
        } else {
            None
        }
    });
    let page_class = if on_thread_route {
        "dm-page is-thread"
    } else {
        "dm-page is-inbox"
    };

    rsx! {
        crate::styles::StyleOnce { id: PAGE_CSS_ID, css: PAGE_CSS }
        div { class: page_class,
            div { class: "dm-layout",
                aside { class: "dm-inbox", aria_label: "Conversations",
                    ConversationList {
                        conversations: convs.clone(),
                        load_state: list_state(&state),
                        members: member_list.clone(),
                        selected_peer: open_peer.clone(),
                        title_is_h1: !on_thread_route,
                        failure,
                        on_retry: move |_| refresh += 1,
                        refreshing: syncing(),
                        notice: sync_notice(),
                        on_refresh: on_refresh,
                        on_compose: on_compose,
                    }
                }
                div { class: "dm-thread-pane",
                    {match open_peer.as_ref() {
                        None => {
                            match inbox_pane_message(&state, convs.len()) {
                                Some(msg) => rsx! { p { class: "dm-thread-empty", "{msg}" } },
                                None => rsx! { div { class: "dm-thread-empty", aria_hidden: "true" } },
                            }
                        }
                        Some(peer) => {
                            let label = route_peer_label.clone().unwrap_or_else(|| label_for(peer));
                            rsx! {
                                MessageThread {
                                    key: "{peer}",
                                    peer_pubkey: peer.clone(),
                                    peer_label: label,
                                    self_label: self_label.clone(),
                                    self_pubkey: self_pubkey.clone(),
                                    heading_is_h1: on_thread_route,
                                    on_read: on_read,
                                }
                            }
                        }
                    }}
                }
            }

            DmComposeModal {
                open: compose_open(),
                self_pubkey: self_pubkey.clone(),
                members: member_list.clone(),
                on_close: on_compose_close,
                on_sent: on_compose_sent,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::admin_shared::test_page::render_admin_route;
    use crate::components::ui::test_support::assert_sizes_from_tokens;
    use crate::state::{RelayAvailability, set_test_relay_availability};
    use scuffed_types::OrgRole;

    struct ClearRelay;
    impl Drop for ClearRelay {
        fn drop(&mut self) {
            set_test_relay_availability(None);
        }
    }

    fn render(path: &str, role: Option<OrgRole>) -> String {
        let _clear = ClearRelay;
        set_test_relay_availability(Some(RelayAvailability::On));
        render_admin_route(path, role)
    }

    fn main_of(html: &str) -> &str {
        let start = html.find("<main").expect("main landmark");
        let end = html[start..].find("</main>").expect("main end") + start;
        &html[start..end]
    }

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
            "use_page_title(",
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

    #[test]
    fn inbox_pane_hides_empty_copy_until_ready() {
        assert_eq!(inbox_pane_message(&LoadState::Loading, 0), None);
        assert_eq!(
            inbox_pane_message(&LoadState::Error(DmLoadFailure::Membership), 0),
            None
        );
        assert_eq!(
            inbox_pane_message(&LoadState::Error(DmLoadFailure::RelayConfig), 3),
            None
        );
        // The inbox's empty card says it; the pane does not repeat it.
        assert_eq!(inbox_pane_message(&LoadState::Ready, 0), None);
        assert_eq!(
            inbox_pane_message(&LoadState::Ready, 2),
            Some("Pick a conversation to read it.")
        );
    }

    #[test]
    fn refresh_toast_counts_stored_messages_in_words() {
        assert_eq!(synced_toast(0), None);
        assert_eq!(synced_toast(1).as_deref(), Some("1 new message."));
        assert_eq!(synced_toast(3).as_deref(), Some("3 new messages."));
    }

    fn convo(pk: &str) -> ConversationSummary {
        ConversationSummary {
            peer_pubkey: pk.to_string(),
            peer_display_name: None,
            last_message_preview: String::new(),
            last_message_at: "2026-11-10T19:00:00Z".to_string(),
            last_sender_pubkey: String::new(),
            unread_count: 1,
        }
    }

    #[test]
    fn the_only_conversation_opens_on_desktop_only() {
        let one = vec![convo(&"ab".repeat(32))];
        assert_eq!(auto_open_peer(&one, true), Some("ab".repeat(32)));
        assert_eq!(auto_open_peer(&one, false), None);
        let two = vec![convo(&"ab".repeat(32)), convo(&"cd".repeat(32))];
        assert_eq!(auto_open_peer(&two, true), None);
        assert_eq!(auto_open_peer(&[], true), None);
    }

    #[test]
    fn dm_sync_stays_closed_until_the_relay_is_configured() {
        use crate::state::RelayAvailability;
        use crate::state::relay::dm_may_sync;

        assert!(!dm_may_sync(RelayAvailability::Off, true));
        assert!(!dm_may_sync(RelayAvailability::Unknown, true));
        assert!(!dm_may_sync(RelayAvailability::Failed, true));
        assert!(!dm_may_sync(RelayAvailability::On, false));
        assert!(dm_may_sync(RelayAvailability::On, true));

        let src = include_str!("dm.rs");
        let start = src
            .find("let _load_conversations")
            .expect("conversation resource");
        let end = src.find("if auth().loading").expect("auth loading return");
        let body = &src[start..end];
        let gate = body.find("dm_may_sync").expect("relay gate");
        let sync = body.find("/api/nostr/dm/sync").expect("sync request");
        assert!(gate < sync, "sync must not run before the relay gate");
        let conversations = body
            .find("/api/nostr/dm/conversations")
            .expect("conversation list");
        assert!(
            gate < conversations,
            "the inbox list must not load before the relay gate"
        );
    }

    #[test]
    fn conversation_fetch_does_not_await_sync() {
        let src = include_str!("dm.rs");
        let start = src
            .find("let _load_conversations")
            .expect("conversation resource");
        let body = &src[start..];
        let fetch_at = body
            .find("/api/nostr/dm/conversations")
            .expect("conversations fetch");
        let before_fetch = &body[..fetch_at];
        let spawn_at = before_fetch
            .find("spawn(async")
            .expect("sync must be spawned so a hung relay does not block the list");
        let sync_at = before_fetch
            .find(".post_json::<_, SyncResponse>")
            .expect("sync still runs");
        assert!(
            sync_at > spawn_at,
            "sync must run inside the spawn, not as an await before the list fetch"
        );
    }

    #[test]
    fn inbox_is_a_full_height_tool_with_one_h1_and_a_labelled_wait() {
        let html = render("/dm", Some(OrgRole::Member));
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(
            !html.contains("site-footer"),
            "full-height tools have no footer: {html}"
        );
        let main = main_of(&html);
        assert_eq!(main.matches("<h1").count(), 1, "{main}");
        assert!(main.contains(">Messages</h1>"), "{main}");
        assert!(main.contains("class=\"dm-page is-inbox\""), "{main}");
        assert!(
            main.contains("role=\"status\">Loading conversations...</p>"),
            "{main}"
        );
        assert!(main.contains("aria-label=\"New message\""), "{main}");
    }

    #[test]
    fn thread_route_names_the_peer_in_the_h1() {
        let pk = "ab".repeat(32);
        let html = render(&format!("/dm/{pk}"), Some(OrgRole::Member));
        let main = main_of(&html);
        assert_eq!(main.matches("<h1").count(), 1, "{main}");
        assert!(main.contains("class=\"dm-page is-thread\""), "{main}");
        assert!(main.contains(">Messages</h2>"), "{main}");
        assert!(main.contains("dm-thread-peer"), "{main}");
        assert!(main.contains("Back to conversations"), "{main}");
        assert!(
            main.contains("role=\"status\">Loading messages...</p>"),
            "{main}"
        );
        // The composer's field is labelled and its status line is rendered.
        assert!(main.contains("for=\"dm-reply-input\""), "{main}");
        assert!(main.contains("id=\"dm-reply-status\""), "{main}");
    }

    #[test]
    fn guests_get_a_log_in_wall_inside_the_shell() {
        let html = render("/dm", None);
        let main = main_of(&html);
        assert_eq!(main.matches("<h1").count(), 1, "{main}");
        assert!(main.contains("Log in to read your messages"), "{main}");
        assert!(main.contains(">Log in</a>"), "{main}");
    }

    #[test]
    fn page_css_is_on_tokens_and_full_height() {
        assert_sizes_from_tokens(PAGE_CSS);
        assert!(PAGE_CSS.contains("height: calc(100dvh - var(--shell-chrome-height));"));
        assert!(PAGE_CSS.contains("max-width: var(--content-max-full);"));
        let phone = PAGE_CSS
            .split("@media (max-width: 820px)")
            .nth(1)
            .expect("phone block");
        assert!(phone.contains(".dm-page.is-thread .dm-inbox { display: none; }"));
        assert!(phone.contains(".dm-page.is-inbox .dm-thread-pane { display: none; }"));
    }
}
