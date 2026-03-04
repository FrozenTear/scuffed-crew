use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;
use crate::routes::Route;

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

const PAGE_CSS: &str = r#"
    .members-page {
        padding: 3rem 2rem;
        max-width: 1000px;
        margin: 0 auto;
    }
    .members-page-title {
        font-family: 'Bebas Neue', sans-serif;
        font-size: 2.5rem;
        color: var(--text-bright);
        letter-spacing: 3px;
        margin: 0 0 2rem;
    }
    .members-grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
        gap: 1.25rem;
    }
    .member-card {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.5rem;
        text-decoration: none;
        text-align: center;
        transition: border-color 0.2s, transform 0.2s;
        display: block;
    }
    .member-card:hover {
        border-color: var(--accent-soft);
        transform: translateY(-2px);
    }
    .member-avatar {
        width: 72px;
        height: 72px;
        border-radius: 50%;
        margin: 0 auto 1rem;
        overflow: hidden;
        background: var(--bg-surface);
        display: flex;
        align-items: center;
        justify-content: center;
    }
    .member-avatar img {
        width: 100%;
        height: 100%;
        object-fit: cover;
    }
    .member-initials {
        font-family: 'Bebas Neue', sans-serif;
        font-size: 1.4rem;
        color: var(--accent);
        letter-spacing: 2px;
    }
    .member-name {
        font-family: 'Rajdhani', sans-serif;
        font-size: 1.1rem;
        font-weight: 700;
        color: var(--text-bright);
        margin: 0 0 0.5rem;
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
    }
    .member-role-pill.admin {
        background: #ef444433;
        color: #f87171;
    }
    .member-role-pill.officer {
        background: #f9731633;
        color: #f97316;
    }
    .member-bio {
        color: var(--text-secondary);
        font-size: 0.8rem;
        line-height: 1.5;
        margin-top: 0.75rem;
        overflow: hidden;
        display: -webkit-box;
        -webkit-line-clamp: 3;
        -webkit-box-orient: vertical;
    }
    .members-loading, .members-empty {
        color: var(--text-muted);
        text-align: center;
        padding: 3rem 0;
    }
"#;

#[component]
pub fn Members() -> Element {
    let members = use_resource(|| async {
        ApiClient::web().fetch::<Vec<PublicMember>>("/api/public/members").await.ok()
    });

    rsx! {
        style { {PAGE_CSS} }

        main { class: "members-page",
            h1 { class: "members-page-title", "Our Crew" }

            {
                let data = members.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! { p { class: "members-loading", "Loading..." } },
                    Some(list) if list.is_empty() => rsx! {
                        p { class: "members-empty", "No members yet." }
                    },
                    Some(list) => rsx! {
                        div { class: "members-grid",
                            for m in list.iter() {
                                {render_member_card(m)}
                            }
                        }
                    },
                }
            }
        }
    }
}

fn render_member_card(m: &PublicMember) -> Element {
    let initials: String = m.display_name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    let role_class = format!("member-role-pill {}", m.org_role);
    let bio = m.bio.clone().unwrap_or_default();

    rsx! {
        Link { to: Route::MemberProfile { id: m.id.clone() }, class: "member-card",
            div { class: "member-avatar",
                if let Some(url) = &m.avatar_url {
                    img { src: "{url}", alt: "{m.display_name}" }
                } else {
                    span { class: "member-initials", "{initials}" }
                }
            }
            h3 { class: "member-name", "{m.display_name}" }
            span { class: "{role_class}", "{m.org_role}" }
            if !bio.is_empty() {
                p { class: "member-bio", "{bio}" }
            }
        }
    }
}
