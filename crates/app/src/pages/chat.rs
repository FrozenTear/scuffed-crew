//! Team chat page (`/chat`), the `d1-chat` frame.
//!
//! A full-height tool: the shell hides the footer on this route and publishes
//! `--shell-chrome-height` (the top bar, plus the tab bar on phone). The page
//! takes the rest of the viewport, the message list scrolls inside it and the
//! composer stays in view. Mounts `ChatWidget` against F-API-003/004.

use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use scuffed_types::{MeResponse, Team, TeamChannel};
use serde::Deserialize;

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

/// The teams the signed-in member is rostered on, as far as the page knows.
/// A team room is readable by its roster only (the relay read gate checks
/// `plays_on`), and a public send is refused off the roster (`send-public`
/// checks it too), whatever the org role.
#[derive(Clone, Debug, PartialEq)]
enum OwnTeams {
    /// The roster answer: ids of the member's teams (empty: on no team).
    Known(Vec<String>),
    /// The lookup failed. The page then behaves as it did before it asked:
    /// every team is offered and none is called out of reach.
    Unknown,
}

impl OwnTeams {
    fn ids(&self) -> &[String] {
        match self {
            Self::Known(ids) => ids,
            Self::Unknown => &[],
        }
    }

    /// The member is known not to be on this team's roster.
    fn excludes(&self, team_id: &str) -> bool {
        match self {
            Self::Known(ids) => !ids.iter().any(|id| id == team_id),
            Self::Unknown => false,
        }
    }
}

/// The part of `GET /api/public/members/{id}` this page reads: the member's
/// active roster entries. `teams` is required: a profile without it is a
/// contract drift, which lands in `OwnTeams::Unknown` and hides no room.
#[derive(Debug, Deserialize)]
struct OwnTeamsProfile {
    teams: Vec<OwnTeamEntry>,
}

#[derive(Debug, Deserialize)]
struct OwnTeamEntry {
    team_id: String,
}

/// The session's member id from `/api/auth/me`, then that member's teams
/// from the public profile (the one existing read of a member's roster
/// entries). A session with no member row is on no team.
async fn load_own_teams() -> OwnTeams {
    let me = match ApiClient::web().fetch::<MeResponse>("/api/auth/me").await {
        Ok(me) => me,
        Err(e) => {
            crate::util::log_client_error("chat me", &e);
            return OwnTeams::Unknown;
        }
    };
    let Some(member) = me.member else {
        return OwnTeams::Known(Vec::new());
    };
    let path = format!(
        "/api/public/members/{}",
        crate::util::encode_query(&member.id)
    );
    match ApiClient::web().fetch::<OwnTeamsProfile>(&path).await {
        Ok(profile) => OwnTeams::Known(profile.teams.into_iter().map(|t| t.team_id).collect()),
        Err(e) => {
            crate::util::log_client_error("chat own teams", &e);
            OwnTeams::Unknown
        }
    }
}

/// The picker order: the member's own teams, then the rest. Each half keeps
/// the order of `list` (by name, as `/api/teams` sends it).
fn own_first<'a>(list: &'a [Team], own: &[String]) -> (Vec<&'a Team>, Vec<&'a Team>) {
    list.iter().partition(|team| own.contains(&team.id))
}

/// The team the page opens: the first of the member's own teams. A member
/// on no listed team still gets the first team picked, so the picker and the
/// title agree; the body then says that room is out of reach.
fn default_team<'a>(list: &'a [Team], own: &[String]) -> Option<&'a str> {
    let (mine, others) = own_first(list, own);
    mine.first().or(others.first()).map(|team| team.id.as_str())
}

/// Group labels of the picker, shown when the member has teams on both sides.
const OWN_TEAMS_LABEL: &str = "Your teams";
const OTHER_TEAMS_LABEL: &str = "Other teams";

fn team_option(team: &Team, selected: Option<&str>) -> Element {
    rsx! {
        option {
            key: "{team.id}",
            value: "{team.id}",
            selected: selected == Some(team.id.as_str()),
            "{team.name}"
        }
    }
}

/// The picker. Each option carries `selected`, so a re-render never shows
/// the first team while the room is another's (the `value` attribute alone
/// is set before the options exist). The member's own teams come first.
fn team_picker(
    list: &[Team],
    own: &[String],
    selected: Option<&str>,
    on_pick: EventHandler<String>,
) -> Element {
    let (mine, others) = own_first(list, own);
    let grouped = !mine.is_empty() && !others.is_empty();
    rsx! {
        div { class: "chat-thread__team",
            label { class: "field-label", r#for: TEAM_SELECT_ID, "Team" }
            select {
                id: TEAM_SELECT_ID,
                class: "field-input",
                value: selected.unwrap_or_default(),
                onchange: move |evt| on_pick.call(evt.value()),
                if grouped {
                    optgroup { label: OWN_TEAMS_LABEL,
                        for team in mine.iter() {
                            {team_option(team, selected)}
                        }
                    }
                    optgroup { label: OTHER_TEAMS_LABEL,
                        for team in others.iter() {
                            {team_option(team, selected)}
                        }
                    }
                } else {
                    for team in mine.iter().chain(others.iter()) {
                        {team_option(team, selected)}
                    }
                }
            }
        }
    }
}

/// What the page shows in place of a room the member is not rostered on: no
/// socket, no empty message list and no composer that can only answer 403.
/// `can_add_self`: officers and admins edit rosters (`POST
/// /api/teams/{id}/roster` takes an officer), so they can add themselves.
fn off_roster_card(team_name: &str, on_any_team: bool, can_add_self: bool) -> Element {
    let message = if on_any_team {
        format!(
            "Only players on the {team_name} roster can read and post in this chat. \
             Pick one of your teams above."
        )
    } else {
        let next = if can_add_self {
            "Add yourself to a roster in team admin."
        } else {
            "An officer can add you to a roster."
        };
        format!(
            "Only players on the {team_name} roster can read and post in this chat, \
             and you are not on a team yet. {next}"
        )
    };
    rsx! {
        EmptyCard { title: "You are not on this team", message }
    }
}

/// What fills the body under the head bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BodyKind {
    /// The team list is not in yet: the member gate on that fetch.
    TeamsPending,
    /// The session is known to hold no org membership: the member gate's
    /// wall (log in, or apply), never a roster card.
    MemberWall,
    NoTeams,
    /// The team list is in, the roster answer is not.
    LoadingOwnTeams,
    /// The picked team is known not to be the member's.
    OffRoster {
        on_any_team: bool,
        can_add_self: bool,
    },
    /// The room, or the member gate on its channels fetch (`room_body`).
    Room,
}

/// The body for a state of the page. The roster card waits for
/// `channels_loaded`: that fetch takes an active org member, so until it
/// has loaded the member gate answers first (an inactive or suspended member
/// gets its wall, not the roster card).
fn body_kind(
    list: Option<&[Team]>,
    selected: Option<&str>,
    own: Option<&OwnTeams>,
    auth: &AuthState,
    channels_loaded: bool,
) -> BodyKind {
    let Some(list) = list else {
        return BodyKind::TeamsPending;
    };
    if !auth.loading && !auth.is_org_member() {
        return BodyKind::MemberWall;
    }
    if list.is_empty() {
        return BodyKind::NoTeams;
    }
    match (selected, own) {
        (None, None) => BodyKind::LoadingOwnTeams,
        (Some(team_id), Some(own)) if channels_loaded && own.excludes(team_id) => {
            BodyKind::OffRoster {
                on_any_team: !own.ids().is_empty(),
                can_add_self: auth.is_officer_or_above(),
            }
        }
        _ => BodyKind::Room,
    }
}

/// The status line of the roster lookup. It is in the head bar in every
/// state; only its text changes, so the wait is announced.
const LOADING_OWN_TEAMS: &str = "Loading your teams.";

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
    // `None` until the answer is in: the default team waits for it.
    let own_teams = use_resource(move || async move {
        if relay.current().allows_connection() {
            Some(load_own_teams().await)
        } else {
            None
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
        // Not before the roster answer: the first team by name is the wrong
        // room for most members.
        if let Some(list) = teams.data.read().as_ref().and_then(|d| d.as_ref())
            && let Some(own) = own_teams.read().as_ref().and_then(|o| o.as_ref())
            && let Some(team) = default_team(list, own.ids())
        {
            selected_team.set(Some(team.to_string()));
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
    let own_data = own_teams.read();
    let own = own_data.as_ref().and_then(|o| o.as_ref());
    let own_ids = own.map(OwnTeams::ids).unwrap_or_default();
    let channels_loaded = channels.data.read().as_ref().is_some_and(Option::is_some);
    let auth_now = auth();
    let kind = body_kind(
        team_list.map(Vec::as_slice),
        selected.as_deref(),
        own,
        &auth_now,
        channels_loaded,
    );
    let own_status = if kind == BodyKind::LoadingOwnTeams {
        LOADING_OWN_TEAMS
    } else {
        ""
    };

    // The picker shows once there is a team to pick. The cap notice is a live
    // region, so it is in the bar in every state.
    let toolbar = rsx! {
        if let Some(list) = team_list.filter(|list| !list.is_empty()) {
            {team_picker(list, own_ids, selected.as_deref(), EventHandler::new(move |id: String| {
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
        p { class: "visually-hidden", role: "status", "{own_status}" }
    };

    let (body, live) = match kind {
        BodyKind::TeamsPending => (
            state_body(member_pending(&auth_now, &teams, "teams")),
            false,
        ),
        BodyKind::MemberWall => (
            state_body(member_pending(&auth_now, &channels, "team channels")),
            false,
        ),
        BodyKind::NoTeams => (
            state_body(rsx! {
                EmptyCard {
                    title: "No teams yet",
                    message: "No teams exist yet, so there are no chat channels to open.",
                }
            }),
            false,
        ),
        // The head bar's status line speaks; this copy is for the eye only.
        BodyKind::LoadingOwnTeams => (
            state_body(rsx! {
                p { class: "chat-thread__note", aria_hidden: "true", "{LOADING_OWN_TEAMS}" }
            }),
            false,
        ),
        BodyKind::OffRoster {
            on_any_team,
            can_add_self,
        } => (
            state_body(off_roster_card(&title, on_any_team, can_add_self)),
            false,
        ),
        BodyKind::Room => room_body(&auth_now, selected, &channels, room),
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
            team_picker(&teams, &[], Some("team:a"), EventHandler::new(|_| {})),
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
            team_picker(&teams, &[], Some("team:b"), EventHandler::new(|_| {}))
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

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    /// Issue 816: the page opened the first team by name, a room most members can
    /// neither read nor post in.
    #[test]
    fn the_default_team_is_one_the_member_is_on() {
        let teams = vec![
            team("team:a", "Squad Alpha"),
            team("team:b", "Squad Bravo"),
            team("team:c", "Squad Charlie"),
        ];
        assert_eq!(default_team(&teams, &ids(&["team:c"])), Some("team:c"));
        assert_eq!(
            default_team(&teams, &ids(&["team:c", "team:b"])),
            Some("team:b"),
            "the first own team in list order"
        );
        assert_eq!(
            default_team(&teams, &ids(&["team:gone", "team:b"])),
            Some("team:b"),
            "a roster entry for a team not in the list is skipped"
        );
        assert_eq!(
            default_team(&teams, &[]),
            Some("team:a"),
            "on no team: the first team, which the body marks out of reach"
        );
        assert_eq!(default_team(&teams, &ids(&["team:gone"])), Some("team:a"));
        assert_eq!(default_team(&[], &ids(&["team:a"])), None);
    }

    #[test]
    fn own_teams_come_first_and_both_halves_keep_list_order() {
        let teams = vec![
            team("team:a", "Squad Alpha"),
            team("team:b", "Squad Bravo"),
            team("team:c", "Squad Charlie"),
            team("team:d", "Squad Delta"),
        ];
        let (mine, others) = own_first(&teams, &ids(&["team:d", "team:b"]));
        let names = |half: &[&Team]| half.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
        assert_eq!(names(&mine), ["team:b", "team:d"]);
        assert_eq!(names(&others), ["team:a", "team:c"]);
    }

    #[test]
    fn only_a_known_roster_puts_a_team_out_of_reach() {
        let own = OwnTeams::Known(ids(&["team:b"]));
        assert!(own.excludes("team:a"));
        assert!(!own.excludes("team:b"));
        assert!(OwnTeams::Known(Vec::new()).excludes("team:a"));
        assert!(
            !OwnTeams::Unknown.excludes("team:a"),
            "a failed lookup hides no room"
        );
        assert!(OwnTeams::Unknown.ids().is_empty());
    }

    #[test]
    fn own_teams_are_read_from_the_public_profile_shape() {
        let profile: OwnTeamsProfile = serde_json::from_str(
            r#"{"id":"m1","display_name":"Ana","teams":[
                {"team_id":"b","team_name":"Squad Bravo","team_role":"player"}
            ],"game_accounts":[]}"#,
        )
        .expect("profile");
        let own: Vec<String> = profile.teams.into_iter().map(|t| t.team_id).collect();
        assert_eq!(own, ["b"]);
        assert!(
            serde_json::from_str::<OwnTeamsProfile>("{}").is_err(),
            "a profile with no teams key is a contract drift (Unknown), not on no team"
        );
        let none: OwnTeamsProfile = serde_json::from_str(r#"{"teams":[]}"#).expect("no teams");
        assert!(none.teams.is_empty());
    }

    #[test]
    fn the_picker_groups_own_teams_first() {
        fn probe() -> Element {
            let teams = vec![
                team("team:a", "Squad Alpha"),
                team("team:b", "Squad Bravo"),
                team("team:c", "Squad Charlie"),
            ];
            let own = vec!["team:c".to_string()];
            team_picker(&teams, &own, Some("team:c"), EventHandler::new(|_| {}))
        }
        let mut dom = VirtualDom::new(probe);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        let mine = html
            .find("<optgroup label=\"Your teams\">")
            .unwrap_or_else(|| panic!("{html}"));
        let others = html
            .find("<optgroup label=\"Other teams\">")
            .unwrap_or_else(|| panic!("{html}"));
        let charlie = html.find("Squad Charlie").expect("own team");
        let alpha = html.find("Squad Alpha").expect("other team");
        assert!(
            mine < charlie && charlie < others && others < alpha,
            "{html}"
        );
        assert!(
            html.contains("<option value=\"team:c\" selected=true>Squad Charlie</option>"),
            "{html}"
        );
        assert_eq!(html.matches("selected=true").count(), 1, "{html}");
    }

    #[test]
    fn the_picker_is_flat_when_there_is_one_group() {
        fn render(own: &'static [&'static str]) -> String {
            thread_local! {
                static OWN: std::cell::Cell<&'static [&'static str]> =
                    const { std::cell::Cell::new(&[]) };
            }
            fn probe() -> Element {
                let teams = vec![team("team:a", "Squad Alpha"), team("team:b", "Squad Bravo")];
                let own = ids(OWN.with(|o| o.get()));
                team_picker(&teams, &own, Some("team:a"), EventHandler::new(|_| {}))
            }
            OWN.with(|o| o.set(own));
            let mut dom = VirtualDom::new(probe);
            dom.rebuild_in_place();
            dioxus_ssr::render(&dom)
        }
        for own in [&[][..], &["team:a", "team:b"][..]] {
            let html = render(own);
            assert!(!html.contains("<optgroup"), "{own:?}: {html}");
            assert_eq!(html.matches("<option").count(), 2, "{html}");
        }
    }

    #[test]
    fn a_room_out_of_reach_says_so_instead_of_mounting_the_room() {
        fn probe() -> Element {
            rsx! {
                {off_roster_card("Squad Alpha", true, false)}
                {off_roster_card("Squad Alpha", false, false)}
                {off_roster_card("Squad Alpha", false, true)}
            }
        }
        let mut dom = VirtualDom::new(probe);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert_eq!(
            html.matches("<h2>You are not on this team</h2>").count(),
            3,
            "{html}"
        );
        assert!(
            html.contains(
                "Only players on the Squad Alpha roster can read and post in this chat. \
                 Pick one of your teams above."
            ),
            "{html}"
        );
        assert_eq!(
            html.matches("you are not on a team yet").count(),
            2,
            "{html}"
        );
        assert_eq!(
            html.matches("An officer can add you to a roster.").count(),
            1,
            "{html}"
        );
        assert_eq!(
            html.matches("Add yourself to a roster in team admin.")
                .count(),
            1,
            "an officer edits rosters: {html}"
        );

        let src = include_str!("chat.rs");
        let prod = src.split("#[cfg(test)]").next().expect("production source");
        assert!(
            prod.contains("let (body, live) = match kind {"),
            "the page renders the body `body_kind` chose"
        );
        assert!(
            prod.contains("&& let Some(team) = default_team(list, own.ids())"),
            "the default team comes from the roster answer"
        );
        assert!(
            !prod.contains("list.first()"),
            "the first team by name is not the default"
        );
    }

    fn auth_as(role: Option<scuffed_types::OrgRole>) -> AuthState {
        AuthState {
            user: Some(scuffed_types::UserInfo {
                id: "u1".into(),
                username: "Player01".into(),
                avatar_url: None,
                role,
            }),
            loading: false,
        }
    }

    /// A row of the body table: case, team list, pick, roster answer, auth,
    /// channels loaded, and the body wanted.
    type BodyCase<'a> = (
        &'a str,
        Option<&'a [Team]>,
        Option<&'a str>,
        Option<&'a OwnTeams>,
        &'a AuthState,
        bool,
        BodyKind,
    );

    /// The body for each state of the page. Review of issue 816: an account
    /// with no member row, or an inactive member, got the roster card in
    /// place of the member gate's wall.
    #[test]
    fn the_body_follows_the_page_state_and_the_member_gate_answers_first() {
        use scuffed_types::OrgRole;
        let teams = vec![team("a", "Squad Alpha"), team("b", "Squad Bravo")];
        let list = Some(teams.as_slice());
        let member = auth_as(Some(OrgRole::Member));
        let officer = auth_as(Some(OrgRole::Officer));
        let no_member_row = auth_as(None);
        let signed_out = AuthState {
            user: None,
            loading: false,
        };
        let auth_loading = AuthState::new();
        let mine = OwnTeams::Known(ids(&["b"]));
        let on_none = OwnTeams::Known(Vec::new());
        let unknown = OwnTeams::Unknown;
        let off_roster = |on_any_team, can_add_self| BodyKind::OffRoster {
            on_any_team,
            can_add_self,
        };

        #[rustfmt::skip]
        let table: Vec<BodyCase> = vec![
            ("teams pending", None, None, None, &member, false, BodyKind::TeamsPending),
            ("teams pending, no member row", None, None, None, &no_member_row, false, BodyKind::TeamsPending),
            ("no teams", Some(&teams[..0]), None, None, &member, false, BodyKind::NoTeams),
            ("roster pending", list, None, None, &member, false, BodyKind::LoadingOwnTeams),
            ("roster pending, auth loading", list, None, None, &auth_loading, false, BodyKind::LoadingOwnTeams),
            ("unknown roster: the room", list, Some("a"), Some(&unknown), &member, true, BodyKind::Room),
            ("own pick: the room", list, Some("b"), Some(&mine), &member, true, BodyKind::Room),
            ("own pick, channels pending", list, Some("b"), Some(&mine), &member, false, BodyKind::Room),
            ("off-roster pick", list, Some("a"), Some(&mine), &member, true, off_roster(true, false)),
            ("off-roster, on no team", list, Some("a"), Some(&on_none), &member, true, off_roster(false, false)),
            ("off-roster officer, on no team", list, Some("a"), Some(&on_none), &officer, true, off_roster(false, true)),
            ("off-roster officer, on a team", list, Some("a"), Some(&mine), &officer, true, off_roster(true, true)),
            // An inactive or suspended member keeps its role in /api/auth/me;
            // the channels fetch (OrgMember) then fails, so the gate answers.
            ("off-roster, channels not loaded", list, Some("a"), Some(&on_none), &member, false, BodyKind::Room),
            ("no member row", list, Some("a"), Some(&on_none), &no_member_row, false, BodyKind::MemberWall),
            ("no member row, roster pending", list, None, None, &no_member_row, false, BodyKind::MemberWall),
            ("no member row, no teams", Some(&teams[..0]), None, None, &no_member_row, false, BodyKind::MemberWall),
            ("signed out", list, Some("a"), Some(&on_none), &signed_out, false, BodyKind::MemberWall),
        ];
        for (case, list, selected, own, auth, channels_loaded, want) in table {
            assert_eq!(
                body_kind(list, selected, own, auth, channels_loaded),
                want,
                "{case}"
            );
        }
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
            team_picker(&teams, &[], Some(picked), EventHandler::new(|_| {})),
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
