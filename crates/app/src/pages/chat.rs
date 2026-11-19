//! Team chat page (`/chat`), the `d1-chat` frame.
//!
//! A full-height tool: the shell hides the footer on this route and publishes
//! `--shell-chrome-height` (the top bar, plus the tab bar on phone). The page
//! takes the rest of the viewport, the message list scrolls inside it and the
//! composer stays in view. Mounts `ChatWidget` against F-API-003/004.

use dioxus::prelude::*;
use scuffed_types::{Team, TeamChannel};

use crate::components::chat::ThreadHead;
use crate::components::chat::thread::thread_label;
use crate::components::ui::EmptyCard;
use crate::components::{list_cap_notice, member_pending};
use crate::hooks::{ApiResource, use_api_list_with, use_api_with};
use crate::pages::chat_gate::ChatGateNotice;
use crate::state::auth::AuthState;
use crate::state::{use_auth, use_relay_availability_state};

#[cfg(feature = "web")]
use crate::components::chat::ChatWidget;
#[cfg(feature = "web")]
use crate::components::chat::chat_widget::RoomLink;
#[cfg(feature = "web")]
use crate::components::chat::relay_status::RelayStatus;
#[cfg(feature = "web")]
use crate::state::nostr::RelayConnectionState;

/// The page body is not a `PageShell`: a tool has no page gutters and no
/// content column (`--content-max-full`). The second `height` is for
/// browsers with dynamic viewport units, so a phone's own toolbars do not
/// push the composer off screen. The floor keeps the head bar, a few lines of
/// messages and the composer usable on a very short window; the page scrolls
/// then.
const PAGE_CSS: &str = r#"
.team-chat {
  box-sizing: border-box;
  width: 100%;
  max-width: var(--content-max-full);
  min-width: 0;
  height: calc(100vh - var(--shell-chrome-height));
  height: calc(100dvh - var(--shell-chrome-height));
  min-height: calc(var(--hit-target-min) * 6);
  display: flex;
  flex-direction: column;
  background: var(--bg);
  color: var(--text);
}
"#;

/// H1 before a team is picked, and when the picked team is not in the list.
const PAGE_TITLE: &str = "Team chat";

/// The one team picker of the page.
const TEAM_SELECT_ID: &str = "team-chat-team";

/// The room name: the picked team, as the frame draws it.
fn room_title(teams: Option<&[Team]>, selected: Option<&str>) -> String {
    teams
        .zip(selected)
        .and_then(|(teams, id)| teams.iter().find(|team| team.id == id))
        .map(|team| team.name.clone())
        .unwrap_or_else(|| PAGE_TITLE.to_string())
}

/// The picker. Each option carries `selected`, so a re-render never shows
/// the first team while the room is another's (the `value` attribute alone
/// is set before the options exist).
fn team_picker(list: &[Team], selected: Option<&str>, on_pick: EventHandler<String>) -> Element {
    rsx! {
        div { class: "chat-thread__team",
            label { class: "field-label", r#for: TEAM_SELECT_ID, "Team" }
            select {
                id: TEAM_SELECT_ID,
                class: "field-input",
                value: selected.unwrap_or_default(),
                onchange: move |evt| on_pick.call(evt.value()),
                for team in list.iter() {
                    option {
                        key: "{team.id}",
                        value: "{team.id}",
                        selected: selected == Some(team.id.as_str()),
                        "{team.name}"
                    }
                }
            }
        }
    }
}

/// The thread: one region and one head bar for every state of the page.
/// Only `body` changes between states and teams, so the head bar and its
/// picker stay mounted (and keep focus) when the room is swapped.
fn thread(title: String, status: Option<Element>, toolbar: Element, body: Element) -> Element {
    let label = thread_label(&title);
    rsx! {
        section { class: "chat-thread", aria_label: "{label}",
            ThreadHead { title, status, toolbar }
            {body}
        }
    }
}

#[component]
pub fn TeamChat() -> Element {
    let relay = use_relay_availability_state();
    let availability = relay.current();
    let auth = use_auth();
    // Empty URL skips the fetch until health says a relay is configured.
    let teams = use_api_list_with::<Team>(move || {
        if relay.current().allows_connection() {
            "/api/teams".to_string()
        } else {
            String::new()
        }
    });
    let mut selected_team = use_signal(|| Option::<String>::None);
    #[cfg(feature = "web")]
    let room = RoomLink {
        state: use_signal(|| RelayConnectionState::Connecting),
        retry: use_signal(|| 0u32),
    };
    #[cfg(not(feature = "web"))]
    let room: RoomArg = ();

    let channels = use_api_with::<Vec<TeamChannel>>(move || {
        if !relay.current().allows_connection() {
            return String::new();
        }
        selected_team()
            .map(|id| format!("/api/teams/{id}/channels"))
            .unwrap_or_default()
    });

    use_effect(move || {
        if selected_team().is_some() {
            return;
        }
        if let Some(list) = teams.data.read().as_ref().and_then(|d| d.as_ref())
            && let Some(first) = list.first()
        {
            selected_team.set(Some(first.id.clone()));
        }
    });

    if !availability.allows_connection() {
        return rsx! {
            ChatGateNotice {
                availability,
                reason: relay.failure_reason(),
                on_retry: move |_| relay.retry(),
            }
        };
    }

    let teams_data = teams.data.read();
    let team_list = teams_data.as_ref().and_then(|d| d.as_ref());
    let selected = selected_team();
    let title = room_title(team_list.map(Vec::as_slice), selected.as_deref());

    // The picker shows once there is a team to pick. The cap notice is a live
    // region, so it is in the bar in every state.
    let toolbar = rsx! {
        if let Some(list) = team_list.filter(|list| !list.is_empty()) {
            {team_picker(list, selected.as_deref(), EventHandler::new(move |id: String| {
                if selected_team.peek().as_deref() == Some(id.as_str()) {
                    return;
                }
                // The next room has not dialled yet.
                #[cfg(feature = "web")]
                {
                    let mut state = room.state;
                    state.set(RelayConnectionState::Connecting);
                }
                selected_team.set(Some(id));
            }))}
        }
        {list_cap_notice(&teams, "teams")}
    };

    let (body, live) = match team_list {
        None => (state_body(member_pending(&auth(), &teams, "teams")), false),
        Some(list) if list.is_empty() => (
            state_body(rsx! {
                EmptyCard {
                    title: "No teams yet",
                    message: "No teams exist yet, so there are no chat channels to open.",
                }
            }),
            false,
        ),
        Some(_) => room_body(&auth(), selected, &channels, room),
    };
    #[cfg(feature = "web")]
    let status = live.then(|| {
        let mut retry = room.retry;
        rsx! {
            RelayStatus {
                state: (room.state)(),
                on_retry: move |_| retry += 1,
            }
        }
    });
    #[cfg(not(feature = "web"))]
    let status: Option<Element> = {
        let _ = live;
        None
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-chat-page-css", css: PAGE_CSS }
        div { class: "team-chat",
            {thread(title, status, toolbar, body)}
        }
    }
}

/// A state with no live room: a wall, an empty card or a status line where
/// the messages will be.
fn state_body(body: Element) -> Element {
    rsx! {
        div { class: "chat-thread__body", {body} }
    }
}

#[cfg(feature = "web")]
type RoomArg = RoomLink;
#[cfg(not(feature = "web"))]
type RoomArg = ();

/// The body under the head bar, and whether it is a live room (whose
/// connection the head bar then shows).
fn room_body(
    auth: &AuthState,
    selected: Option<String>,
    channels: &ApiResource<Vec<TeamChannel>>,
    room: RoomArg,
) -> (Element, bool) {
    let Some(team_key) = selected else {
        return (
            state_body(rsx! {
                p { class: "chat-thread__note", "Select a team to load channels." }
            }),
            false,
        );
    };
    match channels.data.read().as_ref().and_then(|d| d.as_ref()) {
        None => (
            state_body(member_pending(auth, channels, "team channels")),
            false,
        ),
        Some(ch) => {
            let active: Vec<TeamChannel> = ch.iter().filter(|c| c.is_active).cloned().collect();
            if active.is_empty() {
                (
                    state_body(rsx! {
                        EmptyCard {
                            title: "No chat channels",
                            message: "No active chat channels for this team. \
                                An admin can provision them from team settings.",
                        }
                    }),
                    false,
                )
            } else {
                team_chat_panel(team_key, active, room)
            }
        }
    }
}

/// The room is keyed by team: another team is another socket and another
/// message list. Only this body remounts; the head bar above it does not.
fn team_chat_panel(team_key: String, channels: Vec<TeamChannel>, room: RoomArg) -> (Element, bool) {
    #[cfg(feature = "web")]
    {
        (
            rsx! {
                ChatWidget { key: "{team_key}", channels, room }
            },
            true,
        )
    }
    #[cfg(not(feature = "web"))]
    {
        let _ = (team_key, channels, room);
        (
            state_body(rsx! {
                p { class: "chat-thread__note", "Team chat is available in the web app." }
            }),
            false,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{assert_sizes_from_tokens, css_rule, has_decl};

    fn team(id: &str, name: &str) -> Team {
        Team {
            id: id.to_string(),
            name: name.to_string(),
            game_id: "game:ow".to_string(),
            color: None,
            division: None,
            lore_quote: None,
            logo_url: None,
            is_active: true,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn the_room_is_named_after_the_picked_team() {
        let teams = vec![team("team:a", "Squad Alpha"), team("team:b", "Squad Bravo")];
        assert_eq!(
            room_title(Some(teams.as_slice()), Some("team:b")),
            "Squad Bravo"
        );
        assert_eq!(
            room_title(Some(teams.as_slice()), Some("team:a")),
            "Squad Alpha"
        );
        assert_eq!(room_title(Some(teams.as_slice()), None), PAGE_TITLE);
        assert_eq!(
            room_title(Some(teams.as_slice()), Some("team:gone")),
            PAGE_TITLE
        );
        assert_eq!(room_title(None, Some("team:a")), PAGE_TITLE);
        assert_eq!(room_title(Some(&teams[..0]), None), "Team chat");
    }

    fn empty_state() -> Element {
        let teams = vec![team("team:a", "Squad Alpha")];
        thread(
            "Squad Alpha".to_string(),
            None,
            team_picker(&teams, Some("team:a"), EventHandler::new(|_| {})),
            state_body(rsx! {
                EmptyCard {
                    title: "No chat channels",
                    message: "No active chat channels for this team.",
                }
            }),
        )
    }

    #[test]
    fn the_picked_team_is_the_selected_option() {
        fn probe() -> Element {
            let teams = vec![team("team:a", "Squad Alpha"), team("team:b", "Squad Bravo")];
            team_picker(&teams, Some("team:b"), EventHandler::new(|_| {}))
        }
        let mut dom = VirtualDom::new(probe);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert!(
            html.contains("<option value=\"team:b\" selected=true>Squad Bravo</option>"),
            "{html}"
        );
        assert!(
            html.contains("<option value=\"team:a\">Squad Alpha</option>"),
            "only the picked team is selected: {html}"
        );
    }

    thread_local! {
        static PICKED: std::cell::Cell<&'static str> = const { std::cell::Cell::new("team:a") };
    }

    /// The page's shape: one thread, the picker in its head bar, and a body
    /// keyed by team the way `team_chat_panel` keys the room.
    fn switching_probe() -> Element {
        let picked = PICKED.with(|p| p.get());
        let teams = vec![team("team:a", "Squad Alpha"), team("team:b", "Squad Bravo")];
        let title = room_title(Some(teams.as_slice()), Some(picked));
        thread(
            title,
            None,
            team_picker(&teams, Some(picked), EventHandler::new(|_| {})),
            rsx! {
                ProbeRoom { key: "{picked}", picked }
            },
        )
    }

    /// Stands in for the keyed `ChatWidget`.
    #[component]
    fn ProbeRoom(picked: String) -> Element {
        rsx! {
            div { class: "probe-room", "{picked}" }
        }
    }

    #[test]
    fn switching_team_remounts_the_room_but_not_the_picker() {
        use dioxus::dioxus_core::{Mutation, Mutations};
        PICKED.with(|p| p.set("team:a"));
        let mut dom = VirtualDom::new(switching_probe);
        let mut first = Mutations::default();
        dom.rebuild(&mut first);
        let select = first
            .edits
            .iter()
            .find_map(|m| match m {
                Mutation::NewEventListener { name, id } if name == "change" => Some(*id),
                _ => None,
            })
            .expect("the picker listens for change");

        PICKED.with(|p| p.set("team:b"));
        dom.mark_dirty(ScopeId::APP);
        let next = dom.render_immediate_to_vec();
        assert!(
            next.edits
                .iter()
                .any(|m| matches!(m, Mutation::ReplaceWith { .. })),
            "the keyed room is replaced: {:?}",
            next.edits
        );
        assert!(
            !next.edits.iter().any(|m| matches!(
                m,
                Mutation::NewEventListener { name, .. } if name == "change"
            )),
            "a new picker was mounted, so focus would drop: {:?}",
            next.edits
        );
        for edit in &next.edits {
            match edit {
                Mutation::ReplaceWith { id, .. } | Mutation::Remove { id } => {
                    assert_ne!(*id, select, "the picker was remounted: {:?}", next.edits)
                }
                Mutation::RemoveEventListener { id, .. } => {
                    assert_ne!(
                        *id, select,
                        "the picker lost its listener: {:?}",
                        next.edits
                    )
                }
                _ => {}
            }
        }
        assert!(
            next.edits.iter().any(|m| matches!(
                m,
                Mutation::SetAttribute { name: "selected", id, .. } if *id != select
            )),
            "the options follow the pick: {:?}",
            next.edits
        );
        PICKED.with(|p| p.set("team:a"));
    }

    #[test]
    fn a_state_with_no_room_keeps_the_head_bar_and_one_h1() {
        let mut dom = VirtualDom::new(empty_state);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert!(
            html.contains("<section class=\"chat-thread\" aria-label=\"Squad Alpha chat\">"),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Squad Alpha</h1>"), "{html}");
        assert!(!html.contains("<main"), "{html}");
        assert!(
            html.contains("<label class=\"field-label\" for=\"team-chat-team\">Team</label>"),
            "the picker has a visible label: {html}"
        );
        assert!(
            html.contains("<select id=\"team-chat-team\" class=\"field-input\""),
            "{html}"
        );
        assert!(html.contains("<h2>No chat channels</h2>"), "{html}");
        let head = html.find("chat-thread__head").expect("head");
        let body = html.find("chat-thread__body").expect("body");
        assert!(head < body, "{html}");
    }

    #[test]
    fn the_page_fills_the_viewport_under_the_shell_chrome() {
        assert_sizes_from_tokens(crate::components::chat::thread::THREAD_CSS);
        let page = css_rule(PAGE_CSS, ".team-chat {");
        for decl in [
            "height: calc(100vh - var(--shell-chrome-height))",
            "height: calc(100dvh - var(--shell-chrome-height))",
            "max-width: var(--content-max-full)",
            "display: flex",
            "flex-direction: column",
            "min-width: 0",
        ] {
            assert!(has_decl(&page, decl), "missing {decl}: {page}");
        }
        assert!(
            page.find("100vh").unwrap() < page.find("100dvh").unwrap(),
            "the dynamic unit wins where it is supported: {page}"
        );
        assert!(!PAGE_CSS.contains("px"), "{PAGE_CSS}");
        assert!(!PAGE_CSS.contains("rem"), "{PAGE_CSS}");
        assert!(!PAGE_CSS.contains("padding"), "a tool has no page gutters");
    }

    #[test]
    fn page_source_has_no_main_and_no_second_h1() {
        let src = include_str!("chat.rs");
        let prod = src.split("mod tests").next().expect("production source");
        assert!(
            !prod.contains("main {"),
            "the shell owns <main id=\"main\">"
        );
        assert!(!prod.contains("h1 {"), "the H1 comes from ThreadHead");
        assert!(prod.contains("ChatWidget { key: \"{team_key}\", channels, room }"));
        assert_eq!(
            prod.matches("ThreadHead {").count(),
            1,
            "one head bar, outside the keyed room"
        );
        let widget = include_str!("../components/chat/chat_widget.rs");
        let widget = widget.split("mod tests").next().expect("production source");
        assert!(!widget.contains("ThreadHead"), "the room draws no head bar");
        assert!(
            !widget.contains("toolbar"),
            "the picker is not inside the room"
        );
    }
}
