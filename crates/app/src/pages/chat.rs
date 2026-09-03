//! Team chat page — mounts the previously unmounted ChatWidget against F-API-003/004.

use dioxus::prelude::*;
use scuffed_types::{Team, TeamChannel};

use crate::components::member_pending;
use crate::hooks::{ApiResource, use_api_list, use_api_with};
use crate::state::auth::AuthState;
use crate::state::use_auth;

#[cfg(feature = "web")]
use crate::components::chat::ChatWidget;

const PAGE_CSS: &str = r#"
.team-chat-page {
    padding: 2rem;
    max-width: 880px;
    margin: 0 auto;
    box-sizing: border-box;
}
.team-chat-title {
    font-family: var(--font-head);
    font-size: 2.25rem;
    color: var(--text);
    letter-spacing: 3px;
    margin: 0 0 0.35rem;
}
.team-chat-sub {
    color: var(--text-2);
    font-size: 0.9rem;
    margin: 0 0 1.5rem;
}
.team-chat-toolbar {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    flex-wrap: wrap;
    margin-bottom: 1rem;
}
.team-chat-label {
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--text-2);
    text-transform: uppercase;
    letter-spacing: 0.04em;
}
.team-chat-select {
    min-width: 200px;
    padding: 0.45rem 0.7rem;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
    color: var(--text);
    font-family: inherit;
    font-size: 0.9rem;
}
.team-chat-select:focus {
    outline: none;
    border-color: var(--accent);
}
.team-chat-empty {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 2rem 1.5rem;
    color: var(--text-2);
    text-align: center;
}
@media (max-width: 640px) {
    .team-chat-page { padding: 1.25rem 1rem; }
    .team-chat-title { font-size: 1.75rem; }
}
"#;

#[component]
pub fn TeamChat() -> Element {
    let auth = use_auth();
    let teams = use_api_list::<Team>("/api/teams");
    let mut selected_team = use_signal(|| Option::<String>::None);

    let channels = use_api_with::<Vec<TeamChannel>>(move || {
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

    let teams_data = teams.data.read();
    let team_list = teams_data.as_ref().and_then(|d| d.as_ref());

    rsx! {
        style { {PAGE_CSS} }
        main { class: "team-chat-page",
            h1 { class: "team-chat-title", "Team Chat" }
            p { class: "team-chat-sub",
                "Public messages use the team's discovered NIP-29 group. \
                Officer channels send via the server encrypt API."
            }

            {match team_list {
                None => member_pending(&auth(), &teams, "teams"),
                Some(list) if list.is_empty() => rsx! {
                    div { class: "team-chat-empty",
                        "No teams exist yet, so there are no chat channels to open."
                    }
                },
                Some(list) => rsx! {
                    div { class: "team-chat-toolbar",
                        label { class: "team-chat-label", r#for: "team-chat-team", "Team" }
                        select {
                            id: "team-chat-team",
                            class: "team-chat-select",
                            value: selected_team().unwrap_or_default(),
                            onchange: move |evt| selected_team.set(Some(evt.value())),
                            for team in list.iter() {
                                option { value: "{team.id}", "{team.name}" }
                            }
                        }
                    }
                    {channel_panel(&auth(), selected_team(), &channels)}
                },
            }}
        }
    }
}

fn channel_panel(
    auth: &AuthState,
    selected: Option<String>,
    channels: &ApiResource<Vec<TeamChannel>>,
) -> Element {
    if selected.is_none() {
        return rsx! {
            div { class: "team-chat-empty", "Select a team to load channels." }
        };
    }
    match channels.data.read().as_ref().and_then(|d| d.as_ref()) {
        None => member_pending(auth, channels, "team channels"),
        Some(ch) => {
            let active: Vec<TeamChannel> = ch.iter().filter(|c| c.is_active).cloned().collect();
            let team_key = selected.unwrap_or_default();
            if active.is_empty() {
                rsx! {
                    div { class: "team-chat-empty",
                        "No active chat channels for this team. \
                        An admin can provision them from team settings."
                    }
                }
            } else {
                team_chat_panel(team_key, active)
            }
        }
    }
}

fn team_chat_panel(team_key: String, channels: Vec<TeamChannel>) -> Element {
    #[cfg(feature = "web")]
    {
        rsx! {
            ChatWidget {
                key: "{team_key}",
                channels: channels,
                embedded: true,
            }
        }
    }
    #[cfg(not(feature = "web"))]
    {
        let _ = (team_key, channels);
        rsx! {
            div { class: "team-chat-empty",
                "Team chat is available in the web app."
            }
        }
    }
}
