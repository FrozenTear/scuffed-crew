use dioxus::prelude::*;

use super::failure::DmFailureNotice;
use super::types::{
    ConversationSummary, DmLoadFailure, DmMember, avatar_initials, peer_label, relative_time,
};
use crate::components::ui::{EmptyCard, SkeletonBars};
use crate::routes::Route;

/// Id of the header's New button. The compose dialog gives focus back to it.
pub const NEW_MESSAGE_BUTTON_ID: &str = "dm-new-message";
/// The inbox's always-rendered status line.
pub const INBOX_STATUS_ID: &str = "dm-inbox-status";

/// The inbox column of the `d1-dm` frame: a head with the title and New, then
/// one row per conversation (avatar, name, preview, time and unread count).
const CONVERSATION_LIST_CSS: &str = r#"
.dm-inbox-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-inline-start: auto;
}
.dm-inbox-list {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    margin: 0;
    padding: 0;
    list-style: none;
}
.dm-inbox-list > li { margin: 0; }
.dm-inbox-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
}
.dm-inbox-body .empty-card {
    margin: var(--space-4);
    padding: var(--space-card-pad);
}
.dm-inbox-body .empty-card h2 { font-size: var(--text-xl); }
.dm-inbox-body .ui-skel-list { padding: var(--space-4); }
.dm-convo {
    box-sizing: border-box;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    gap: var(--space-3);
    align-items: center;
    min-height: max(var(--hit-target-min), var(--density-row));
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--border);
    color: var(--text);
    text-decoration: none;
}
.dm-convo:hover { background: var(--surface-raised); }
.dm-convo.is-current,
.dm-convo[aria-current="page"] { background: var(--accent-soft); }
.dm-convo:focus-visible {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: calc(var(--focus-ring-width) * -1);
}
.dm-convo-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
.dm-convo-who {
    font-family: var(--font-head);
    font-size: var(--text-md);
    font-weight: var(--font-label-weight);
}
.dm-convo-preview {
    color: var(--text-3);
    font-size: var(--text-sm);
}
.dm-convo-meta {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--space-1);
    color: var(--text-3);
    font-size: var(--text-sm);
    white-space: nowrap;
}
.dm-unread {
    box-sizing: border-box;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: var(--space-5);
    height: var(--space-5);
    padding: 0 var(--space-1);
    border-radius: var(--radius-pill);
    background: var(--accent-solid);
    color: var(--on-accent);
    font-family: var(--font-head);
    font-size: var(--badge-font-size);
    font-weight: var(--badge-font-weight);
    text-transform: var(--badge-font-transform);
}
"#;

/// Inbox list phase. Empty copy is only for [`ConversationListState::Ready`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConversationListState {
    Loading,
    Ready,
    Failed,
}

fn shows_empty_inbox(load_state: ConversationListState, len: usize) -> bool {
    load_state == ConversationListState::Ready && len == 0
}

/// Text of the inbox's status line. It is always rendered (only the text
/// changes), so the change is announced. A failure has its own alert.
fn inbox_status_text(load_state: ConversationListState, len: usize) -> String {
    match load_state {
        ConversationListState::Loading => "Loading conversations...".to_string(),
        ConversationListState::Failed => String::new(),
        ConversationListState::Ready => match len {
            0 => "No conversations yet.".to_string(),
            1 => "1 conversation.".to_string(),
            n => format!("{n} conversations."),
        },
    }
}

/// The count to show on a row. The open conversation is being read, so it
/// shows none even before the server's mark-read lands.
fn shown_unread(c: &ConversationSummary, current: Option<&str>) -> u64 {
    if current == Some(c.peer_pubkey.as_str()) {
        0
    } else {
        c.unread_count
    }
}

#[component]
pub fn ConversationList(
    conversations: Vec<ConversationSummary>,
    load_state: ConversationListState,
    /// Crew members, to show peers by name.
    #[props(default)]
    members: Vec<DmMember>,
    /// The open conversation (from the address, or the only one on desktop).
    selected_peer: Option<String>,
    /// True on `/dm`: the column title is the page's H1. On a thread the
    /// peer's name is the H1 and this title is an H2.
    #[props(default)]
    title_is_h1: bool,
    #[props(default)] failure: Option<DmLoadFailure>,
    #[props(default)] on_retry: Option<EventHandler<()>>,
    refreshing: bool,
    on_refresh: EventHandler<()>,
    on_compose: EventHandler<()>,
) -> Element {
    let empty = shows_empty_inbox(load_state, conversations.len());
    let status = inbox_status_text(load_state, conversations.len());
    // Header and empty-state each own a callback so compose stays available
    // after the first conversation exists. EventHandler is Copy.
    let compose_from_header = on_compose;
    let compose_from_empty = on_compose;
    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-components-dm-conversation_list-conversation-list-css", css: CONVERSATION_LIST_CSS }
        div { class: "dm-pane-head dm-inbox-head",
            if title_is_h1 {
                h1 { class: "dm-pane-title", "Messages" }
            } else {
                h2 { class: "dm-pane-title", "Messages" }
            }
            div { class: "dm-inbox-actions",
                button {
                    id: NEW_MESSAGE_BUTTON_ID,
                    class: "btn is-primary",
                    r#type: "button",
                    "aria-label": "New message",
                    "aria-haspopup": "dialog",
                    onclick: move |_| compose_from_header.call(()),
                    "New"
                }
                button {
                    class: "btn is-ghost",
                    r#type: "button",
                    aria_disabled: if refreshing { "true" },
                    onclick: move |_| {
                        if !refreshing {
                            on_refresh.call(());
                        }
                    },
                    if refreshing { "Syncing..." } else { "Refresh" }
                }
            }
        }
        p { id: INBOX_STATUS_ID, class: "visually-hidden", role: "status", "{status}" }
        if let Some(failure) = failure {
            div { class: "dm-inbox-body",
                DmFailureNotice {
                    failure,
                    title: "Could not load your conversations",
                    on_retry,
                }
            }
        } else if conversations.is_empty() {
            div { class: "dm-inbox-body",
                if empty {
                    EmptyCard {
                        title: "No conversations yet",
                        message: "Messages you send or get from crew members show up here. Start one with New message.",
                        actions: rsx! {
                            button {
                                class: "btn is-primary",
                                r#type: "button",
                                "aria-haspopup": "dialog",
                                onclick: move |_| compose_from_empty.call(()),
                                "New message"
                            }
                        },
                    }
                } else if load_state == ConversationListState::Loading {
                    SkeletonBars { rows: 4 }
                }
            }
        } else {
            ul { class: "dm-inbox-list", aria_label: "Conversations",
                for c in conversations.iter() {
                    li { key: "{c.peer_pubkey}",
                        {render_row(c, selected_peer.as_deref(), &members)}
                    }
                }
            }
        }
    }
}

fn render_row(c: &ConversationSummary, selected: Option<&str>, members: &[DmMember]) -> Element {
    let display_name = peer_label(&c.peer_pubkey, c.peer_display_name.as_deref(), members);
    let initials = avatar_initials(&display_name);
    let time = relative_time(&c.last_message_at);
    let is_current = selected == Some(c.peer_pubkey.as_str());
    let cls = if is_current {
        "dm-convo is-current"
    } else {
        "dm-convo"
    };
    let unread = shown_unread(c, selected);
    let preview = c.last_message_preview.trim().to_string();

    rsx! {
        Link {
            to: Route::DmThread { peer_pubkey: c.peer_pubkey.clone() },
            class: "{cls}",
            span { class: "dm-avatar", aria_hidden: "true", "{initials}" }
            span { class: "dm-convo-text",
                span { class: "dm-convo-who", "{display_name}" }
                if !preview.is_empty() {
                    span { class: "dm-convo-preview", " {preview}" }
                }
            }
            span { class: "dm-convo-meta",
                time { datetime: "{c.last_message_at}", "{time}" }
                if unread > 0 {
                    span { class: "dm-unread",
                        "{unread}"
                        span { class: "visually-hidden", " unread" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{assert_sizes_from_tokens, render_routed};

    fn convo(pk: &str, unread: u64) -> ConversationSummary {
        ConversationSummary {
            peer_pubkey: pk.to_string(),
            peer_display_name: None,
            last_message_preview: "Sure, after 20:00.".to_string(),
            last_message_at: "2026-11-10T19:00:00Z".to_string(),
            last_sender_pubkey: String::new(),
            unread_count: unread,
        }
    }

    #[test]
    fn empty_inbox_copy_only_after_ready() {
        assert!(!shows_empty_inbox(ConversationListState::Loading, 0));
        assert!(!shows_empty_inbox(ConversationListState::Failed, 0));
        assert!(shows_empty_inbox(ConversationListState::Ready, 0));
        assert!(!shows_empty_inbox(ConversationListState::Ready, 2));
    }

    #[test]
    fn status_line_text_follows_the_load() {
        assert_eq!(
            inbox_status_text(ConversationListState::Loading, 0),
            "Loading conversations..."
        );
        assert_eq!(inbox_status_text(ConversationListState::Failed, 0), "");
        assert_eq!(
            inbox_status_text(ConversationListState::Ready, 0),
            "No conversations yet."
        );
        assert_eq!(
            inbox_status_text(ConversationListState::Ready, 1),
            "1 conversation."
        );
        assert_eq!(
            inbox_status_text(ConversationListState::Ready, 3),
            "3 conversations."
        );
    }

    #[test]
    fn the_open_conversation_shows_no_unread_badge() {
        let open = convo(&"ab".repeat(32), 3);
        let other = convo(&"cd".repeat(32), 2);
        let current = Some(open.peer_pubkey.as_str());
        assert_eq!(shown_unread(&open, current), 0);
        assert_eq!(shown_unread(&other, current), 2);
        assert_eq!(shown_unread(&open, None), 3);
    }

    fn list(
        conversations: Vec<ConversationSummary>,
        load_state: ConversationListState,
        failure: Option<DmLoadFailure>,
    ) -> String {
        render_routed(move || {
            let conversations = conversations.clone();
            let failure = failure.clone();
            rsx! {
                ConversationList {
                    conversations,
                    load_state,
                    members: vec![DmMember {
                        id: "member:p2".into(),
                        display_name: "Player02".into(),
                        nostr_pubkey: Some("ab".repeat(32)),
                    }],
                    selected_peer: None,
                    title_is_h1: true,
                    failure,
                    on_retry: move |_| {},
                    refreshing: false,
                    on_refresh: move |_| {},
                    on_compose: move |_| {},
                }
            }
        })
    }

    #[test]
    fn status_line_is_rendered_in_every_state() {
        let marker = format!("id=\"{INBOX_STATUS_ID}\"");
        for (convs, state, failure) in [
            (vec![], ConversationListState::Loading, None),
            (vec![], ConversationListState::Ready, None),
            (
                vec![],
                ConversationListState::Failed,
                Some(DmLoadFailure::RelayConfig),
            ),
            (
                vec![convo(&"ab".repeat(32), 1)],
                ConversationListState::Ready,
                None,
            ),
        ] {
            let html = list(convs, state, failure);
            assert_eq!(html.matches(&marker).count(), 1, "{html}");
            assert!(html.contains("role=\"status\""), "{html}");
        }
    }

    #[test]
    fn rows_show_the_member_name_not_the_key() {
        let html = list(
            vec![convo(&"ab".repeat(32), 1)],
            ConversationListState::Ready,
            None,
        );
        assert!(html.contains(">Player02<"), "{html}");
        assert!(!html.contains("npub1"), "{html}");
        assert!(html.contains("<span class=\"dm-avatar\" aria-hidden=\"true\">PL</span>"));
        // The count is a word too, not a coloured dot alone.
        assert!(
            html.contains("1<span class=\"visually-hidden\"> unread</span>"),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
    }

    #[test]
    fn empty_inbox_says_what_is_missing_and_offers_new_message() {
        let html = list(vec![], ConversationListState::Ready, None);
        assert!(html.contains("No conversations yet"), "{html}");
        assert!(html.contains(">New message</button>"), "{html}");
    }

    #[test]
    fn a_failed_load_has_its_reason_and_retry() {
        let html = list(
            vec![],
            ConversationListState::Failed,
            Some(DmLoadFailure::Other("HTTP 502: relay unreachable".into())),
        );
        assert!(html.contains("role=\"alert\""), "{html}");
        assert!(html.contains("relay unreachable"), "{html}");
        assert!(html.contains(">Retry</button>"), "{html}");
        assert!(!html.contains("No conversations yet"), "{html}");
    }

    /// The empty-state CTA used to be the only caller of `on_compose`, so the
    /// New message control disappeared as soon as one conversation existed.
    #[test]
    fn new_message_control_lives_in_the_list_header() {
        let src = include_str!("conversation_list.rs");
        let start = src
            .find("pub fn ConversationList")
            .expect("ConversationList");
        let body = &src[start..];
        let empty_at = body
            .find("conversations.is_empty()")
            .expect("empty-state branch");
        let header = &body[..empty_at];
        assert!(
            header.contains("compose_from_header.call"),
            "header must call on_compose; the empty state must not be the only path"
        );
        assert!(
            header.contains("\"aria-label\": \"New message\""),
            "compose control needs an accessible name, not an icon alone"
        );
        assert!(
            header.contains("\"New\""),
            "visible New label must be present in the header"
        );
        let empty = &body[empty_at..];
        assert!(
            empty.contains("compose_from_empty.call"),
            "empty-state CTA should still open compose"
        );
    }

    #[test]
    fn inbox_css_is_on_tokens() {
        assert_sizes_from_tokens(CONVERSATION_LIST_CSS);
    }
}
