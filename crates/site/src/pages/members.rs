use leptos::prelude::*;
use serde::Deserialize;

use scuffed_auth::client::api::fetch_json_list;

use crate::components::Nav;
use crate::sections::Footer;

#[derive(Debug, Clone, Deserialize)]
struct PublicMember {
    id: String,
    display_name: String,
    org_role: String,
    bio: Option<String>,
    avatar_url: Option<String>,
    #[allow(dead_code)]
    joined_at: String,
}

#[component]
pub fn MembersPage() -> impl IntoView {
    let members = LocalResource::new(|| async {
        fetch_json_list::<PublicMember>("/api/public/members").await.ok()
    });

    view! {
        <Nav/>
        <main class="members-page">
            <h1 class="members-page-title">"Our Crew"</h1>

            {move || match members.get().flatten() {
                None => view! { <p class="members-loading">"Loading..."</p> }.into_any(),
                Some(list) if list.is_empty() => view! { <p class="members-empty">"No members yet."</p> }.into_any(),
                Some(list) => view! {
                    <div class="members-grid">
                        {list.into_iter().map(|m| {
                            let href = format!("/members/{}", m.id);
                            let initials: String = m.display_name
                                .split_whitespace()
                                .filter_map(|w| w.chars().next())
                                .take(2)
                                .collect::<String>()
                                .to_uppercase();
                            let role_class = format!("member-role-pill {}", m.org_role);
                            let bio = m.bio.unwrap_or_default();
                            view! {
                                <a href=href class="member-card">
                                    <div class="member-avatar">
                                        {match m.avatar_url {
                                            Some(url) => view! { <img src=url alt=m.display_name.clone()/> }.into_any(),
                                            None => view! { <span class="member-initials">{initials}</span> }.into_any(),
                                        }}
                                    </div>
                                    <h3 class="member-name">{m.display_name}</h3>
                                    <span class=role_class>{m.org_role.clone()}</span>
                                    {(!bio.is_empty()).then(|| view! {
                                        <p class="member-bio">{bio}</p>
                                    })}
                                </a>
                            }
                        }).collect_view()}
                    </div>
                }.into_any(),
            }}
        </main>
        <Footer/>
    }
}
