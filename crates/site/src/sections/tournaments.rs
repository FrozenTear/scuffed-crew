use leptos::prelude::*;
use serde::Deserialize;

use scuffed_auth::client::api::fetch_json;

use crate::components::SectionHeader;
use crate::components::bracket::BRACKET_STYLES;

#[derive(Debug, Clone, Deserialize)]
struct Tournament {
    id: String,
    name: String,
    format: String,
    status: String,
    is_external: bool,
    starts_at: Option<String>,
}

fn format_label(f: &str) -> &str {
    match f {
        "single_elim" => "Single Elim",
        "double_elim" => "Double Elim",
        "round_robin" => "Round Robin",
        "swiss" => "Swiss",
        _ => f,
    }
}

fn status_label(s: &str) -> &str {
    match s {
        "registration" => "Registration Open",
        "in_progress" => "Live",
        "completed" => "Completed",
        _ => s,
    }
}

#[component]
pub fn Tournaments() -> impl IntoView {
    let tournaments = LocalResource::new(|| async {
        fetch_json::<Vec<Tournament>>("/api/tournaments").await.ok()
    });

    view! {
        <leptos_meta::Style>{BRACKET_STYLES}</leptos_meta::Style>
        <section id="tournaments">
            <SectionHeader
                label="// Compete"
                title="Tournaments"
                color="purple"
                description="Active and upcoming tournaments."
            />

            <div class="tournament-cards" data-reveal="">
                {move || {
                    let list = tournaments.get().flatten().unwrap_or_default();
                    let visible: Vec<Tournament> = list.into_iter()
                        .filter(|t| t.status == "registration" || t.status == "in_progress")
                        .take(4)
                        .collect();

                    if visible.is_empty() {
                        return view! {
                            <p style="color: var(--text-muted); text-align: center;">
                                "No active tournaments right now."
                            </p>
                        }.into_any();
                    }

                    view! {
                        <div class="tournament-home-grid">
                            {visible.into_iter().map(|t| {
                                let href = format!("/tournaments/{}", t.id);
                                let format_text = format_label(&t.format).to_string();
                                let status_text = status_label(&t.status).to_string();
                                let status_class = format!("tournament-card-status {}", t.status);
                                let date = t.starts_at.as_ref()
                                    .map(|d| d.chars().take(10).collect::<String>())
                                    .unwrap_or_default();

                                view! {
                                    <a href=href class="tournament-card" data-reveal="">
                                        <div class="tournament-card-name">{t.name}</div>
                                        <div class="tournament-card-meta">
                                            <span>{format_text}</span>
                                            {(!date.is_empty()).then(|| view! { <span>{date}</span> })}
                                            {t.is_external.then(|| view! { <span>"External"</span> })}
                                        </div>
                                        <span class=status_class>{status_text}</span>
                                    </a>
                                }
                            }).collect_view()}
                        </div>
                    }.into_any()
                }}
            </div>

            <div style="text-align: center; margin-top: 1.5rem;" data-reveal="">
                <a href="/tournaments" class="btn btn-secondary">"View All Tournaments"</a>
            </div>
        </section>
    }
}
