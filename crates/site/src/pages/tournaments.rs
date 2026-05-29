use leptos::prelude::*;
use serde::Deserialize;

use scuffed_auth::client::api::fetch_json_list;

use crate::components::Nav;
use crate::components::bracket::BRACKET_STYLES;
use crate::sections::Footer;

#[derive(Debug, Clone, Deserialize)]
struct Tournament {
    id: String,
    name: String,
    #[allow(dead_code)]
    game_id: Option<String>,
    format: String,
    status: String,
    #[allow(dead_code)]
    max_teams: Option<u32>,
    is_external: bool,
    #[allow(dead_code)]
    external_url: Option<String>,
    description: Option<String>,
    starts_at: Option<String>,
    #[allow(dead_code)]
    created_at: String,
}

fn format_label(f: &str) -> &str {
    match f {
        "single_elim" => "Single Elimination",
        "double_elim" => "Double Elimination",
        "round_robin" => "Round Robin",
        "swiss" => "Swiss",
        _ => f,
    }
}

fn status_label(s: &str) -> &str {
    match s {
        "draft" => "Draft",
        "registration" => "Registration Open",
        "in_progress" => "In Progress",
        "completed" => "Completed",
        "archived" => "Archived",
        _ => s,
    }
}

#[component]
pub fn TournamentsPage() -> impl IntoView {
    let tournaments = LocalResource::new(|| async {
        fetch_json_list::<Tournament>("/api/tournaments").await.ok()
    });

    view! {
        <leptos_meta::Style>{BRACKET_STYLES}</leptos_meta::Style>
        <Nav/>
        <main class="tournaments-page">
            <h1>"Tournaments"</h1>

            {move || match tournaments.get().flatten() {
                None => view! { <p >"Loading..."</p> }.into_any(),
                Some(list) if list.is_empty() => view! {
                    <p >"No tournaments yet."</p>
                }.into_any(),
                Some(list) => {
                    // Filter out drafts from public view
                    let visible: Vec<Tournament> = list.into_iter()
                        .filter(|t| t.status != "draft")
                        .collect();
                    if visible.is_empty() {
                        return view! {
                            <p >"No tournaments yet."</p>
                        }.into_any();
                    }
                    view! {
                        <div class="tournament-grid">
                            {visible.into_iter().map(|t| {
                                let href = format!("/tournaments/{}", t.id);
                                let format_text = format_label(&t.format).to_string();
                                let status_text = status_label(&t.status).to_string();
                                let status_class = format!("tournament-card-status {}", t.status);
                                let date = t.starts_at.as_ref()
                                    .map(|d| d.chars().take(10).collect::<String>())
                                    .unwrap_or_default();

                                view! {
                                    <a href=href class="tournament-card">
                                        <div class="tournament-card-name">{t.name}</div>
                                        <div class="tournament-card-meta">
                                            <span>{format_text}</span>
                                            {(!date.is_empty()).then(|| view! { <span>{date}</span> })}
                                            {t.is_external.then(|| view! { <span>"External"</span> })}
                                        </div>
                                        {t.description.map(|d| view! {
                                            <p class="tournament-card-desc">
                                                {d.chars().take(120).collect::<String>()}
                                            </p>
                                        })}
                                        <span class=status_class>{status_text}</span>
                                    </a>
                                }
                            }).collect_view()}
                        </div>
                    }.into_any()
                }
            }}
        </main>
        <Footer/>
    }
}
