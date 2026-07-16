use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::ui::{Card, Pill, PillTone};
use crate::routes::Route;
use scuffed_api_client::ApiClient;

use super::public_fetch::{PublicFetch, fetch_public};

#[derive(Debug, Clone, Deserialize)]
struct MemberTeamInfo {
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
    #[serde(default)]
    rank: Option<String>,
    #[serde(default)]
    sr: Option<u32>,
    #[serde(default)]
    role: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct HeroStats {
    hero: String,
    matches: u32,
    wins: u32,
    losses: u32,
    draws: u32,
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
    #[serde(default)]
    main_role: Option<String>,
    #[serde(default)]
    twitch: Option<String>,
    #[serde(default)]
    twitter: Option<String>,
}

const PAGE_CSS: &str = r#"
    .profile-page {
        padding: 3rem 2rem;
        max-width: 800px;
        margin: 0 auto;
    }
    .profile-loading {
        color: var(--text-3);
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
        background: var(--surface-2);
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
        font-family: var(--font-head);
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
        font-family: var(--font-head);
        font-size: 2.2rem;
        color: var(--text);
        letter-spacing: 2px;
        margin: 0;
        line-height: 1;
    }
    .profile-joined {
        color: var(--text-3);
        font-size: 0.8rem;
        margin: 0;
    }
    .profile-section {
        margin-bottom: 2rem;
    }
    .profile-section h2 {
        font-family: var(--font-head);
        font-size: 1.2rem;
        font-weight: 700;
        color: var(--text);
        margin: 0 0 0.75rem;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .profile-bio {
        color: var(--text-2);
        font-size: 0.9rem;
        line-height: 1.7;
        margin: 0;
    }
    .profile-teams {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
        gap: 0.75rem;
    }
    .profile-team-link {
        display: block;
        color: inherit;
        text-decoration: none;
        min-width: 0;
    }
    .profile-team-link:hover .profile-team-name {
        color: var(--accent);
    }
    .profile-team-name {
        font-family: var(--font-head);
        font-weight: 700;
        color: var(--text);
    }
    .profile-team-role {
        font-size: 0.7rem;
        color: var(--text-3);
        text-transform: uppercase;
    }
    .profile-account-name {
        font-family: var(--font-head);
        font-weight: 700;
        color: var(--text);
    }
    .profile-account-id {
        font-family: var(--font-mono);
        font-size: 0.75rem;
        color: var(--text-3);
    }
    .profile-card-row {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: var(--space-3);
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
    .profile-pills {
        display: flex;
        gap: 0.4rem;
        flex-wrap: wrap;
        align-items: center;
    }
    .profile-socials {
        display: flex;
        gap: 0.9rem;
        margin-top: 0.2rem;
    }
    .profile-socials a {
        color: var(--accent);
        text-decoration: none;
        font-size: 0.8rem;
        font-weight: 600;
    }
    .profile-socials a:hover {
        text-decoration: underline;
    }
    .profile-account-meta {
        display: flex;
        gap: 0.5rem;
        margin-top: 0.4rem;
        font-size: 0.72rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.03em;
    }
    .profile-heroes {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
        gap: 0.75rem;
    }
    .profile-hero-name {
        font-family: var(--font-head);
        font-weight: 700;
        color: var(--text);
        text-transform: capitalize;
    }
    .profile-hero-meta {
        font-size: 0.75rem;
        color: var(--text-3);
        margin-top: 0.25rem;
    }
    @media (max-width: 600px) {
        .profile-header {
            flex-direction: column;
            text-align: center;
        }
        .profile-pills, .profile-socials {
            justify-content: center;
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
                return PublicFetch::NotFound;
            }
            fetch_public::<MemberProfileData>(&format!("/api/public/members/{id}")).await
        }
    });

    // Hero showcase is member-gated until #6 ships a public endpoint: anon
    // (or any failure) collapses to empty and the section stays hidden.
    let hero_id = id.clone();
    let heroes = use_resource(move || {
        let id = hero_id.clone();
        async move {
            if id.is_empty() {
                return Vec::new();
            }
            ApiClient::web()
                .fetch::<Vec<HeroStats>>(&format!("/api/stats/member/{id}/heroes"))
                .await
                .unwrap_or_default()
        }
    });

    rsx! {
        style { {PAGE_CSS} }

        main { class: "profile-page",
            {
                let data = profile.read();
                match data.as_ref() {
                    None => rsx! { p { class: "profile-loading", "Loading..." } },
                    Some(PublicFetch::NotFound) => rsx! { p { class: "profile-loading", "Member not found" } },
                    Some(PublicFetch::Failed) => rsx! {
                        p { class: "profile-loading", "Couldn't load this profile. Check your connection and try again." }
                    },
                    Some(PublicFetch::Found(m)) => {
                        let initials: String = m.display_name
                            .split_whitespace()
                            .filter_map(|w| w.chars().next())
                            .take(2)
                            .collect::<String>()
                            .to_uppercase();
                        let role_tone = match m.org_role.as_str() {
                            "admin" => PillTone::Danger,
                            "officer" => PillTone::Warn,
                            _ => PillTone::Accent,
                        };
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
                                    div { class: "profile-pills",
                                        Pill { tone: role_tone, "{m.org_role}" }
                                        if let Some(role) = &m.main_role {
                                            Pill { tone: PillTone::Accent, "{role}" }
                                        }
                                    }
                                    if m.twitch.is_some() || m.twitter.is_some() {
                                        div { class: "profile-socials",
                                            if let Some(h) = &m.twitch {
                                                a {
                                                    href: "https://twitch.tv/{h}",
                                                    target: "_blank",
                                                    rel: "noopener noreferrer",
                                                    "Twitch"
                                                }
                                            }
                                            if let Some(h) = &m.twitter {
                                                a {
                                                    href: "https://x.com/{h}",
                                                    target: "_blank",
                                                    rel: "noopener noreferrer",
                                                    "Twitter / X"
                                                }
                                            }
                                        }
                                    }
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
                                            Link {
                                                to: Route::TeamPage { id: t.team_id.clone() },
                                                class: "profile-team-link",
                                                Card {
                                                    div { class: "profile-card-row",
                                                        span { class: "profile-team-name", "{t.team_name}" }
                                                        span { class: "profile-team-role", "{t.team_role}" }
                                                    }
                                                }
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

                            {
                                let hero_list = heroes.read().clone().unwrap_or_default();
                                rsx! {
                                    if !hero_list.is_empty() {
                                        div { class: "profile-section",
                                            h2 { "Top Heroes" }
                                            div { class: "profile-heroes",
                                                for h in hero_list.iter().take(3) {
                                                    {render_hero_card(h)}
                                                }
                                            }
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
    let rank = a.rank.clone().unwrap_or_default();
    let sr = a.sr.map(|v| v.to_string()).unwrap_or_default();
    let role = a.role.clone().unwrap_or_default();
    let has_meta = !rank.is_empty() || !sr.is_empty() || !role.is_empty();
    rsx! {
        Card {
            div { class: "profile-card-row",
                span { class: "profile-account-name", "{a.account_name}" }
                if !id_display.is_empty() {
                    span { class: "profile-account-id", "{id_display}" }
                }
            }
            if has_meta {
                div { class: "profile-account-meta",
                    if !rank.is_empty() {
                        span { "{rank}" }
                    }
                    if !sr.is_empty() {
                        span { "{sr} SR" }
                    }
                    if !role.is_empty() {
                        span { "{role}" }
                    }
                }
            }
        }
    }
}

fn render_hero_card(h: &HeroStats) -> Element {
    let winrate = if h.matches > 0 {
        format!("{:.0}%", (h.wins as f64 / h.matches as f64) * 100.0)
    } else {
        "—".to_string()
    };
    rsx! {
        Card {
            div { class: "profile-hero-name", "{h.hero}" }
            div { class: "profile-hero-meta",
                "{h.matches} games · {winrate} WR · {h.wins}-{h.losses}-{h.draws}"
            }
        }
    }
}
