use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::bracket::BRACKET_STYLES;
use crate::routes::Route;
use scuffed_api_client::ApiClient;

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

use crate::hooks::CursorPage;

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

const PAGE_CSS: &str = r#"
    .tournaments-page {
        padding: 3rem 2rem;
        max-width: 1000px;
        margin: 0 auto;
    }
    .tournaments-page h1 {
        font-family: 'Bebas Neue', sans-serif;
        font-size: 2.5rem;
        color: var(--text-bright);
        letter-spacing: 3px;
        margin: 0 0 2rem;
    }
    .tournament-grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
        gap: 1.25rem;
    }
    .tournament-card {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.25rem;
        text-decoration: none;
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
        transition: border-color 0.2s, transform 0.2s;
    }
    .tournament-card:hover {
        border-color: var(--accent-soft);
        transform: translateY(-2px);
    }
    .tournament-card-name {
        font-family: 'Rajdhani', sans-serif;
        font-weight: 700;
        font-size: 1.15rem;
        color: var(--text-bright);
    }
    .tournament-card-meta {
        display: flex;
        gap: 0.75rem;
        flex-wrap: wrap;
        font-size: 0.75rem;
        color: var(--text-muted);
    }
    .tournament-card-desc {
        color: var(--text-secondary);
        font-size: 0.8rem;
        line-height: 1.5;
        margin: 0;
    }
    .tournament-card-status {
        display: inline-block;
        font-size: 0.65rem;
        padding: 0.15rem 0.6rem;
        border-radius: 999px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
        width: fit-content;
    }
    .tournament-card-status.registration {
        background: #10b98133;
        color: #34d399;
    }
    .tournament-card-status.in_progress {
        background: #f9731633;
        color: #f97316;
    }
    .tournament-card-status.completed {
        background: #6b728033;
        color: #9ca3af;
    }
    .tournament-card-status.archived {
        background: #6b728033;
        color: #6b7280;
    }
    .tournaments-loading, .tournaments-empty {
        color: var(--text-muted);
        text-align: center;
        padding: 3rem 0;
    }
"#;

#[component]
pub fn Tournaments() -> Element {
    let tournaments = use_resource(|| async {
        ApiClient::web()
            .fetch::<CursorPage<Tournament>>("/api/tournaments")
            .await
            .ok()
            .map(|r| r.data)
    });

    rsx! {
        style { {PAGE_CSS} }
        style { {BRACKET_STYLES} }

        main { class: "tournaments-page",
            h1 { "Tournaments" }

            {
                let data = tournaments.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! { p { class: "tournaments-loading", "Loading..." } },
                    Some(list) => {
                        let visible: Vec<&Tournament> = list.iter()
                            .filter(|t| t.status != "draft")
                            .collect();
                        if visible.is_empty() {
                            rsx! { p { class: "tournaments-empty", "No tournaments yet." } }
                        } else {
                            rsx! {
                                div { class: "tournament-grid",
                                    for t in visible.iter() {
                                        {render_tournament_card(t)}
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn render_tournament_card(t: &Tournament) -> Element {
    let format_text = format_label(&t.format);
    let status_text = status_label(&t.status);
    let status_class = format!("tournament-card-status {}", t.status);
    let date: String = t
        .starts_at
        .as_ref()
        .map(|d| d.chars().take(10).collect())
        .unwrap_or_default();
    let desc_preview: Option<String> = t
        .description
        .as_ref()
        .map(|d| d.chars().take(120).collect());

    rsx! {
        Link { to: Route::Tournament { id: t.id.clone() }, class: "tournament-card",
            div { class: "tournament-card-name", "{t.name}" }
            div { class: "tournament-card-meta",
                span { "{format_text}" }
                if !date.is_empty() {
                    span { "{date}" }
                }
                if t.is_external {
                    span { "External" }
                }
            }
            if let Some(desc) = &desc_preview {
                p { class: "tournament-card-desc", "{desc}" }
            }
            span { class: "{status_class}", "{status_text}" }
        }
    }
}
