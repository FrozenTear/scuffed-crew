use dioxus::prelude::*;
use serde::Deserialize;

use crate::routes::Route;
use scuffed_api_client::ApiClient;

#[derive(Debug, Clone, Deserialize)]
struct MemberTeamInfo {
    #[allow(dead_code)]
    team_id: String,
    team_name: String,
    team_role: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GameAccount {
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    member_id: String,
    #[allow(dead_code)]
    game_id: String,
    account_name: String,
    account_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MemberProfileData {
    #[allow(dead_code)]
    id: String,
    display_name: String,
    org_role: String,
    bio: Option<String>,
    avatar_url: Option<String>,
    joined_at: String,
    teams: Vec<MemberTeamInfo>,
    #[serde(default)]
    game_accounts: Vec<GameAccount>,
}

const PAGE_CSS: &str = r#"
    .profile-page {
        padding: 3rem 2rem;
        max-width: 800px;
        margin: 0 auto;
    }
    .profile-loading {
        color: var(--text-muted);
        text-align: center;
        padding: 3rem 0;
    }
    .profile-header {
        display: flex;
        align-items: center;
        gap: 2rem;
        margin-bottom: 2.5rem;
    }
    .profile-avatar-large {
        width: 120px;
        height: 120px;
        border-radius: 50%;
        overflow: hidden;
        background: var(--bg-surface);
        display: flex;
        align-items: center;
        justify-content: center;
        flex-shrink: 0;
    }
    .profile-avatar-large img {
        width: 100%;
        height: 100%;
        object-fit: cover;
    }
    .profile-avatar-large .member-initials {
        font-family: 'Bebas Neue', sans-serif;
        font-size: 2.4rem;
        color: var(--accent);
        letter-spacing: 3px;
    }
    .profile-info {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
    }
    .profile-name {
        font-family: 'Bebas Neue', sans-serif;
        font-size: 2.2rem;
        color: var(--text-bright);
        letter-spacing: 2px;
        margin: 0;
        line-height: 1;
    }
    .member-role-pill {
        display: inline-block;
        font-size: 0.65rem;
        padding: 0.15rem 0.6rem;
        border-radius: 999px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
        background: #7c3aed33;
        color: #a78bfa;
        width: fit-content;
    }
    .member-role-pill.admin {
        background: #ef444433;
        color: #f87171;
    }
    .member-role-pill.officer {
        background: #f9731633;
        color: #f97316;
    }
    .profile-joined {
        color: var(--text-muted);
        font-size: 0.8rem;
        margin: 0;
    }
    .profile-section {
        margin-bottom: 2rem;
    }
    .profile-section h2 {
        font-family: 'Rajdhani', sans-serif;
        font-size: 1.2rem;
        font-weight: 700;
        color: var(--text-bright);
        margin: 0 0 0.75rem;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .profile-bio {
        color: var(--text-secondary);
        font-size: 0.9rem;
        line-height: 1.7;
        margin: 0;
    }
    .profile-teams {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
        gap: 0.75rem;
    }
    .profile-team-card {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 1rem;
        display: flex;
        justify-content: space-between;
        align-items: center;
    }
    .profile-team-name {
        font-family: 'Rajdhani', sans-serif;
        font-weight: 700;
        color: var(--text-bright);
    }
    .profile-team-role {
        font-size: 0.7rem;
        color: var(--text-muted);
        text-transform: uppercase;
    }
    .profile-account-card {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 1rem;
        display: flex;
        justify-content: space-between;
        align-items: center;
    }
    .profile-account-name {
        font-family: 'Rajdhani', sans-serif;
        font-weight: 700;
        color: var(--text-bright);
    }
    .profile-account-id {
        font-family: 'DM Mono', monospace;
        font-size: 0.75rem;
        color: var(--text-muted);
    }
    .profile-back {
        margin-top: 2rem;
        padding-top: 1.5rem;
        border-top: 1px solid var(--border);
    }
    .profile-back a {
        color: var(--accent);
        text-decoration: none;
        font-size: 0.85rem;
        font-weight: 600;
    }
    .profile-back a:hover {
        text-decoration: underline;
    }
    @media (max-width: 600px) {
        .profile-header {
            flex-direction: column;
            text-align: center;
        }
    }
"#;

#[component]
pub fn MemberProfile(id: String) -> Element {
    let id_clone = id.clone();
    let profile = use_resource(move || {
        let id = id_clone.clone();
        async move {
            if id.is_empty() {
                return None;
            }
            ApiClient::web()
                .fetch::<MemberProfileData>(&format!("/api/public/members/{id}"))
                .await
                .ok()
        }
    });

    rsx! {
        style { {PAGE_CSS} }

        main { class: "profile-page",
            {
                let data = profile.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! { p { class: "profile-loading", "Loading..." } },
                    Some(m) => {
                        let initials: String = m.display_name
                            .split_whitespace()
                            .filter_map(|w| w.chars().next())
                            .take(2)
                            .collect::<String>()
                            .to_uppercase();
                        let role_class = format!("member-role-pill {}", m.org_role);
                        let joined = m.joined_at.chars().take(10).collect::<String>();
                        let bio = m.bio.clone().unwrap_or_default();

                        rsx! {
                            div { class: "profile-header",
                                div { class: "profile-avatar-large",
                                    if let Some(url) = &m.avatar_url {
                                        img { src: "{url}", alt: "{m.display_name}" }
                                    } else {
                                        span { class: "member-initials", "{initials}" }
                                    }
                                }
                                div { class: "profile-info",
                                    h1 { class: "profile-name", "{m.display_name}" }
                                    span { class: "{role_class}", "{m.org_role}" }
                                    p { class: "profile-joined", "Joined {joined}" }
                                }
                            }

                            if !bio.is_empty() {
                                div { class: "profile-section",
                                    h2 { "About" }
                                    p { class: "profile-bio", "{bio}" }
                                }
                            }

                            if !m.teams.is_empty() {
                                div { class: "profile-section",
                                    h2 { "Teams" }
                                    div { class: "profile-teams",
                                        for t in m.teams.iter() {
                                            div { class: "profile-team-card",
                                                span { class: "profile-team-name", "{t.team_name}" }
                                                span { class: "profile-team-role", "{t.team_role}" }
                                            }
                                        }
                                    }
                                }
                            }

                            if !m.game_accounts.is_empty() {
                                div { class: "profile-section",
                                    h2 { "Game Accounts" }
                                    div { class: "profile-teams",
                                        for a in m.game_accounts.iter() {
                                            {render_account_card(a)}
                                        }
                                    }
                                }
                            }

                            div { class: "profile-back",
                                Link { to: Route::Members {}, "Back to all members" }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn render_account_card(a: &GameAccount) -> Element {
    let id_display = a.account_id.clone().unwrap_or_default();
    rsx! {
        div { class: "profile-account-card",
            span { class: "profile-account-name", "{a.account_name}" }
            if !id_display.is_empty() {
                span { class: "profile-account-id", "{id_display}" }
            }
        }
    }
}
