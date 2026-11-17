use dioxus::prelude::*;

use scuffed_api_client::{ApiClient, ClientError};

use super::failure::DmFailureNotice;
use super::reply_input::ReplyInput;
use super::types::{
    DmLoadFailure, DmMessage, MarkReadBody, avatar_initials, classify_dm_client_error,
    relative_time,
};
use crate::components::ui::{EmptyCard, SkeletonBars};
use crate::routes::Route;
use crate::util::encode_query;

/// The scrolling message list. One thread per page, so a fixed id.
pub const MESSAGE_LIST_ID: &str = "dm-message-list";
/// The thread's always-rendered status line (loading, older messages).
pub const THREAD_STATUS_ID: &str = "dm-thread-status";

/// The thread column of the `d1-dm` frame: a head with the peer, the
/// messages on `--bg` (newest at the bottom, mine on the right on the soft
/// accent), then the composer.
const MESSAGE_THREAD_CSS: &str = r#"
.dm-thread {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
}
.dm-thread-back { display: none; }
.dm-thread-peer {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
.dm-msg-list {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    margin: 0;
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    background: var(--bg);
    list-style: none;
}
.dm-msg-list:focus-visible {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: calc(var(--focus-ring-width) * -1);
}
/* Newest at the bottom. An auto margin, not justify-content: flex-end, so
   the oldest messages can still be scrolled to. */
.dm-msg-list > :first-child { margin-top: auto; }
.dm-msg {
    box-sizing: border-box;
    width: fit-content;
    max-width: min(60ch, 100%);
    margin: 0;
    padding: var(--space-3);
    align-self: flex-start;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    overflow-wrap: anywhere;
}
.dm-msg.is-me {
    align-self: flex-end;
    background: var(--accent-soft);
    border-color: var(--accent-soft);
}
.dm-msg-meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin: 0 0 var(--space-1);
    color: var(--text-3);
    font-size: var(--text-sm);
}
.dm-msg.is-me .dm-msg-meta { color: var(--text); }
.dm-msg-body {
    margin: 0;
    font-size: var(--text-md);
    line-height: var(--leading-body);
    white-space: pre-wrap;
}
.dm-msg-list .dm-older { align-self: center; }
.dm-msg-list .empty-card {
    align-self: stretch;
    padding: var(--space-card-pad);
}
.dm-msg-list .empty-card h2 { font-size: var(--text-xl); }
.dm-msg-list .ui-skel-list { align-self: stretch; }
.dm-msg-list .dm-failure { margin: 0; align-self: stretch; }
"#;

#[component]
pub fn MessageThread(
    peer_pubkey: String,
    /// The peer's name (from the server, the member list, or a short npub).
    peer_label: String,
    /// My own display name, for the meta line of my messages.
    #[props(default)]
    self_label: Option<String>,
    self_pubkey: Option<String>,
    /// True on `/dm/:peer`: the peer's name is the page's H1.
    #[props(default)]
    heading_is_h1: bool,
    /// Fired with the peer key once the thread is loaded and marked read, so
    /// the inbox can clear that row's unread count.
    #[props(default)]
    on_read: Option<EventHandler<String>>,
) -> Element {
    let peer = peer_pubkey.clone();
    let peer_for_resource = peer.clone();
    let self_pk = self_pubkey.clone();

    let mut messages = use_signal::<Vec<DmMessage>>(Vec::new);
    let mut loading_initial = use_signal(|| true);
    let mut load_error = use_signal(|| None::<DmLoadFailure>);
    let mut loading_older = use_signal(|| false);
    let mut has_more = use_signal(|| true);
    let mut last_marked_until = use_signal(|| None::<String>);

    // Load initial page when peer changes.
    let mut load_initial = use_resource(move || {
        let peer = peer_for_resource.clone();
        async move {
            loading_initial.set(true);
            load_error.set(None);
            messages.set(Vec::new());
            has_more.set(true);
            match fetch_thread(&peer, None, 50).await {
                Ok(page) => {
                    if page.len() < 50 {
                        has_more.set(false);
                    }
                    let mut sorted = page;
                    sorted.sort_by(|a, b| a.created_at.cmp(&b.created_at));
                    let until = sorted.last().map(|m| m.created_at.clone());
                    messages.set(sorted);
                    load_error.set(None);
                    loading_initial.set(false);
                    if let Some(ts) = until
                        && last_marked_until() != Some(ts.clone())
                    {
                        mark_read(&peer, &ts).await;
                        last_marked_until.set(Some(ts));
                    }
                    if let Some(on_read) = on_read {
                        on_read.call(peer.clone());
                    }
                }
                Err(err) => {
                    load_error.set(Some(classify_dm_client_error(&err)));
                    loading_initial.set(false);
                }
            }
        }
    });

    // Keep the newest message in view: after the first load and after a send.
    let message_count = messages.read().len();
    use_effect(use_reactive((&message_count,), move |(count,)| {
        if count > 0 && !*loading_older.peek() {
            scroll_to_end(MESSAGE_LIST_ID);
        }
    }));

    let peer_for_older = peer.clone();
    let load_older = move |_| {
        if loading_older() || !has_more() {
            return;
        }
        let oldest = messages.read().first().map(|m| m.created_at.clone());
        let Some(before) = oldest else { return };
        let peer = peer_for_older.clone();
        loading_older.set(true);
        spawn(async move {
            match fetch_thread(&peer, Some(&before), 50).await {
                Ok(page) => {
                    if page.len() < 50 {
                        has_more.set(false);
                    }
                    let mut combined = page;
                    combined.sort_by(|a, b| a.created_at.cmp(&b.created_at));
                    let mut current = messages();
                    combined.append(&mut current);
                    messages.set(combined);
                }
                Err(_) => {
                    has_more.set(false);
                }
            }
            loading_older.set(false);
        });
    };

    let initials = avatar_initials(&peer_label);
    let me_label = self_label
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "You".to_string());

    let msgs = messages();
    let is_loading_initial = loading_initial();
    let failure = load_error();
    let show_empty = thread_shows_empty(is_loading_initial, failure.is_some(), msgs.len());
    let reply_to = msgs.last().map(|m| m.gift_wrap_id.clone());
    let status = thread_status_text(is_loading_initial, loading_older());

    let on_optimistic = move |msg: DmMessage| {
        // Append to the local thread cache so the UI updates without waiting
        // for the next /api/nostr/dm/sync. The unique gift_wrap_id index on
        // the server side dedupes when the next sync re-stores the message.
        let mut current = messages();
        current.push(msg);
        messages.set(current);
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-components-dm-message_thread-message-thread-css", css: MESSAGE_THREAD_CSS }
        section { class: "dm-thread", aria_label: "Conversation with {peer_label}",
            div { class: "dm-pane-head dm-thread-head",
                Link {
                    to: Route::DmInbox {},
                    class: "btn is-ghost dm-thread-back",
                    aria_label: "Back to conversations",
                    "Back"
                }
                span { class: "dm-avatar", aria_hidden: "true", "{initials}" }
                if heading_is_h1 {
                    h1 { class: "dm-pane-title dm-thread-peer", "{peer_label}" }
                } else {
                    h2 { class: "dm-pane-title dm-thread-peer", "{peer_label}" }
                }
            }
            p { id: THREAD_STATUS_ID, class: "visually-hidden", role: "status", "{status}" }
            // A focusable region, so the keyboard can scroll the history.
            div { id: MESSAGE_LIST_ID, class: "dm-msg-list", tabindex: "0", role: "region",
                aria_label: "Messages with {peer_label}",
                if has_more() && !is_loading_initial && !msgs.is_empty() {
                    button {
                        class: "btn is-ghost dm-older",
                        r#type: "button",
                        aria_disabled: if loading_older() { "true" },
                        onclick: load_older,
                        if loading_older() { "Loading..." } else { "Load older messages" }
                    }
                }
                if is_loading_initial {
                    SkeletonBars { rows: 3 }
                } else if let Some(failure) = failure {
                    DmFailureNotice {
                        failure,
                        title: "Could not load this conversation",
                        on_retry: move |_| load_initial.restart(),
                    }
                } else if show_empty {
                    EmptyCard {
                        title: "No messages yet",
                        message: "Write the first message to {peer_label} in the box below.",
                    }
                } else {
                    for m in msgs.iter() {
                        {render_message(m, self_pk.as_deref(), &me_label, &peer_label)}
                    }
                }
            }
            ReplyInput {
                peer_pubkey: peer.clone(),
                peer_label: peer_label.clone(),
                self_pubkey: self_pk.clone(),
                reply_to_event_id: reply_to,
                on_optimistic: on_optimistic,
            }
        }
    }
}

fn render_message(m: &DmMessage, self_pubkey: Option<&str>, me: &str, peer: &str) -> Element {
    let outgoing = self_pubkey
        .map(|pk| pk.eq_ignore_ascii_case(&m.sender_pubkey))
        .unwrap_or(false);
    let cls = if outgoing { "dm-msg is-me" } else { "dm-msg" };
    let who = if outgoing { me } else { peer };
    let time = relative_time(&m.created_at);
    rsx! {
        article { key: "{m.id}", class: "{cls}",
            p { class: "dm-msg-meta",
                span { "{who}" }
                time { datetime: "{m.created_at}", "{time}" }
            }
            p { class: "dm-msg-body", "{m.content}" }
        }
    }
}

/// Text of the thread's status line. Always rendered, only the text changes.
fn thread_status_text(loading_initial: bool, loading_older: bool) -> &'static str {
    if loading_initial {
        "Loading messages..."
    } else if loading_older {
        "Loading older messages..."
    } else {
        ""
    }
}

/// Scroll the list to its end, so the newest message is in view.
fn scroll_to_end(id: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(el) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id(id))
        {
            el.set_scroll_top(el.scroll_height());
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = id;
    }
}

async fn fetch_thread(
    peer: &str,
    before_ts: Option<&str>,
    limit: usize,
) -> Result<Vec<DmMessage>, ClientError> {
    let mut url = format!(
        "/api/nostr/dm/thread?peer_pubkey={}&limit={}",
        encode_query(peer),
        limit
    );
    if let Some(ts) = before_ts {
        url.push_str(&format!("&before_ts={}", encode_query(ts)));
    }
    ApiClient::web().fetch::<Vec<DmMessage>>(&url).await
}

fn thread_shows_empty(loading: bool, failed: bool, len: usize) -> bool {
    !loading && !failed && len == 0
}

async fn mark_read(peer: &str, until_ts: &str) {
    let body = MarkReadBody {
        peer_pubkey: peer.to_string(),
        until_ts: until_ts.to_string(),
    };
    let _ = ApiClient::web()
        .post_json::<_, serde_json::Value>("/api/nostr/dm/mark-read", &body)
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{
        assert_ink_clears_aa, assert_sizes_from_tokens, render_routed,
    };

    #[test]
    fn empty_thread_copy_only_after_a_successful_load() {
        assert!(!thread_shows_empty(true, false, 0));
        assert!(!thread_shows_empty(false, true, 0));
        assert!(thread_shows_empty(false, false, 0));
        assert!(!thread_shows_empty(false, false, 1));
    }

    #[test]
    fn status_line_says_what_is_loading() {
        assert_eq!(thread_status_text(true, false), "Loading messages...");
        assert_eq!(thread_status_text(false, true), "Loading older messages...");
        assert_eq!(thread_status_text(false, false), "");
    }

    #[test]
    fn first_render_is_a_labelled_wait_with_the_peer_as_h1() {
        let html = render_routed(|| {
            rsx! {
                crate::components::ToastProvider {
                    MessageThread {
                        peer_pubkey: "ab".repeat(32),
                        peer_label: "Player02",
                        self_pubkey: None,
                        heading_is_h1: true,
                    }
                }
            }
        });
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains(">Player02</h1>"), "{html}");
        assert!(
            html.contains(&format!(
                "<p id=\"{THREAD_STATUS_ID}\" class=\"visually-hidden\" role=\"status\">Loading messages...</p>"
            )),
            "{html}"
        );
        assert!(
            html.contains("aria-label=\"Back to conversations\""),
            "{html}"
        );
    }

    #[test]
    fn inside_the_inbox_the_peer_is_an_h2() {
        let html = render_routed(|| {
            rsx! {
                crate::components::ToastProvider {
                    MessageThread {
                        peer_pubkey: "ab".repeat(32),
                        peer_label: "Player02",
                        self_pubkey: None,
                    }
                }
            }
        });
        assert!(!html.contains("<h1"), "{html}");
        assert!(html.contains(">Player02</h2>"), "{html}");
    }

    #[test]
    fn messages_name_the_sender_so_mine_is_not_colour_alone() {
        let html = crate::components::ui::test_support::render(|| {
            let m = DmMessage {
                id: "m1".into(),
                gift_wrap_id: "g1".into(),
                sender_pubkey: "AB".repeat(32),
                recipient_pubkey: "cd".repeat(32),
                content: "Hey, want to duo later?".into(),
                created_at: "2026-11-10T19:00:00Z".into(),
                reply_to_event_id: None,
            };
            render_message(&m, Some(&"ab".repeat(32)), "Player01", "Player02")
        });
        assert!(html.contains("class=\"dm-msg is-me\""), "{html}");
        assert!(html.contains("<span>Player01</span>"), "{html}");
        assert!(html.contains("datetime=\"2026-11-10T19:00:00Z\""), "{html}");
    }

    #[test]
    fn thread_css_is_on_tokens_and_readable() {
        assert_sizes_from_tokens(MESSAGE_THREAD_CSS);
        assert_ink_clears_aa("--text", "--accent-soft", 4.5);
        assert_ink_clears_aa("--text-3", "--bg", 4.5);
        assert_ink_clears_aa("--text-3", "--surface", 4.5);
    }
}
