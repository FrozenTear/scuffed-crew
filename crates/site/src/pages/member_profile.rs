use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use serde::Deserialize;

use scuffed_auth::client::api::fetch_json;

use crate::components::Nav;
use crate::sections::Footer;

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
struct MemberProfile {
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

#[component]
pub fn MemberProfilePage() -> impl IntoView {
    let params = use_params_map();

    let profile = LocalResource::new(move || {
        let id = params.get().get("id").unwrap_or_default();
        async move {
            if id.is_empty() {
                return None;
            }
            fetch_json::<MemberProfile>(&format!("/api/public/members/{id}"))
                .await
                .ok()
        }
    });

    view! {
        <Nav/>
        <main class="profile-page">
            {move || match profile.get().flatten() {
                None => view! { <p class="profile-loading">"Loading..."</p> }.into_any(),
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

                    view! {
                        <div class="profile-header">
                            <div class="profile-avatar-large">
                                {match m.avatar_url.clone() {
                                    Some(url) => view! { <img src=url alt=m.display_name.clone()/> }.into_any(),
                                    None => view! { <span class="member-initials">{initials}</span> }.into_any(),
                                }}
                            </div>
                            <div class="profile-info">
                                <h1 class="profile-name">{m.display_name.clone()}</h1>
                                <span class=role_class>{m.org_role.clone()}</span>
                                <p class="profile-joined">"Joined "{joined}</p>
                            </div>
                        </div>

                        {(!bio.is_empty()).then(|| view! {
                            <div class="profile-section">
                                <h2>"About"</h2>
                                <p class="profile-bio">{bio}</p>
                            </div>
                        })}

                        {(!m.teams.is_empty()).then(|| view! {
                            <div class="profile-section">
                                <h2>"Teams"</h2>
                                <div class="profile-teams">
                                    {m.teams.into_iter().map(|t| {
                                        view! {
                                            <div class="profile-team-card">
                                                <span class="profile-team-name">{t.team_name}</span>
                                                <span class="profile-team-role">{t.team_role}</span>
                                            </div>
                                        }
                                    }).collect_view()}
                                </div>
                            </div>
                        })}

                        {(!m.game_accounts.is_empty()).then(|| view! {
                            <div class="profile-section">
                                <h2>"Game Accounts"</h2>
                                <div class="profile-teams">
                                    {m.game_accounts.into_iter().map(|a| {
                                        let id_display = a.account_id.unwrap_or_default();
                                        view! {
                                            <div class="profile-account-card">
                                                <span class="profile-account-name">{a.account_name}</span>
                                                {(!id_display.is_empty()).then(|| view! {
                                                    <span class="profile-account-id">{id_display}</span>
                                                })}
                                            </div>
                                        }
                                    }).collect_view()}
                                </div>
                            </div>
                        })}

                        <div class="profile-back">
                            <a href="/members">"Back to all members"</a>
                        </div>
                    }.into_any()
                }
            }}
        </main>
        <Footer/>
    }
}
