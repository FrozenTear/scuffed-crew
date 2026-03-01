use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use serde::Deserialize;
use std::collections::HashMap;

use scuffed_auth::client::api::fetch_json;

use crate::components::bracket::{BracketMatch, BracketRound, BracketView, SwissStanding, BRACKET_STYLES};
use crate::components::Nav;
use crate::sections::Footer;

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct TournamentDetail {
    id: String,
    name: String,
    game_id: Option<String>,
    format: String,
    status: String,
    max_teams: Option<u32>,
    best_of: u32,
    is_external: bool,
    external_url: Option<String>,
    rules: Option<String>,
    description: Option<String>,
    starts_at: Option<String>,
    ends_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Participant {
    id: String,
    team_id: Option<String>,
    external_name: Option<String>,
    #[allow(dead_code)]
    seed: Option<u32>,
    #[allow(dead_code)]
    status: String,
}

#[derive(Debug, Clone, Deserialize)]
struct BracketData {
    tournament: TournamentDetail,
    participants: Vec<Participant>,
    rounds: Vec<BracketRound>,
    matches: Vec<BracketMatch>,
}

#[derive(Debug, Clone, Deserialize)]
struct Team {
    id: String,
    name: String,
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
        "in_progress" => "Live",
        "completed" => "Completed",
        "archived" => "Archived",
        _ => s,
    }
}

#[component]
pub fn TournamentPage() -> impl IntoView {
    let params = use_params_map();

    let refresh = RwSignal::new(0u32);

    let bracket_data = LocalResource::new(move || {
        refresh.get();
        let id = params.get().get("id").unwrap_or_default();
        async move {
            if id.is_empty() {
                return None;
            }
            fetch_json::<BracketData>(&format!("/api/tournaments/{id}/bracket"))
                .await
                .ok()
        }
    });

    // Fetch teams for name resolution
    let teams = LocalResource::new(|| async {
        fetch_json::<Vec<Team>>("/api/teams").await.ok()
    });

    // Fetch swiss standings if applicable
    let standings = LocalResource::new(move || {
        refresh.get();
        let id = params.get().get("id").unwrap_or_default();
        async move {
            fetch_json::<Vec<SwissStanding>>(&format!("/api/tournaments/{id}/standings"))
                .await
                .ok()
        }
    });

    // Auto-refresh for live tournaments (30s)
    {
        use wasm_bindgen::prelude::*;
        let closure = Closure::wrap(Box::new(move || {
            refresh.update(|n| *n += 1);
        }) as Box<dyn Fn()>);
        if let Some(window) = web_sys::window() {
            let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
                closure.as_ref().unchecked_ref(),
                30_000,
            );
        }
        closure.forget();
    }

    view! {
        <leptos_meta::Style>{BRACKET_STYLES}</leptos_meta::Style>
        <Nav/>
        <main class="tournament-detail-page">
            {move || {
                let team_map: HashMap<String, String> = teams.get().flatten()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|t| (t.id, t.name))
                    .collect();

                match bracket_data.get().flatten() {
                    None => view! { <p style="color: var(--text-muted);">"Loading..."</p> }.into_any(),
                    Some(data) => {
                        let t = &data.tournament;
                        let format_text = format_label(&t.format).to_string();
                        let status_text = status_label(&t.status).to_string();
                        let status_class = format!("tournament-card-status {}", t.status);

                        // Build participant name map
                        let mut pnames: HashMap<String, String> = HashMap::new();
                        let mut pids: Vec<String> = Vec::new();
                        for p in &data.participants {
                            let name = if let Some(ext) = &p.external_name {
                                ext.clone()
                            } else if let Some(tid) = &p.team_id {
                                team_map.get(tid).cloned().unwrap_or_else(|| tid.clone())
                            } else {
                                "TBD".to_string()
                            };
                            pnames.insert(p.id.clone(), name);
                            pids.push(p.id.clone());
                        }

                        let swiss = standings.get().flatten();

                        let name = t.name.clone();
                        let bo = t.best_of;
                        let desc = t.description.clone();
                        let rules = t.rules.clone();
                        let ext_url = t.external_url.clone();
                        let is_ext = t.is_external;
                        let starts = t.starts_at.clone();
                        let format_str = t.format.clone();

                        view! {
                            <div class="tournament-header">
                                <h1>{name}</h1>
                                <div class="tournament-meta">
                                    <span>{format_text}</span>
                                    {(bo > 1).then(|| view! { <span>{format!("Bo{bo}")}</span> })}
                                    <span class=status_class>{status_text}</span>
                                    {starts.map(|d| {
                                        let date = d.chars().take(10).collect::<String>();
                                        view! { <span>{date}</span> }
                                    })}
                                    {is_ext.then(|| view! { <span>"External"</span> })}
                                </div>
                                {desc.map(|d| view! {
                                    <p class="tournament-description">{d}</p>
                                })}
                                {ext_url.map(|url| {
                                    let url2 = url.clone();
                                    view! {
                                        <a href=url target="_blank" class="tournament-external-link">
                                            {url2}
                                        </a>
                                    }
                                })}
                            </div>

                            {if !data.matches.is_empty() {
                                view! {
                                    <div class="tournament-section-title">"Bracket"</div>
                                    <BracketView
                                        format=format_str
                                        rounds=data.rounds
                                        matches=data.matches
                                        participant_names=pnames
                                        participant_ids=pids
                                        swiss_standings=swiss.unwrap_or_default()
                                    />
                                }.into_any()
                            } else {
                                view! {
                                    <p style="color: var(--text-muted);">"Bracket not yet generated."</p>
                                }.into_any()
                            }}

                            {rules.map(|r| view! {
                                <div class="tournament-section-title">"Rules"</div>
                                <p class="tournament-description">{r}</p>
                            })}
                        }.into_any()
                    }
                }
            }}
        </main>
        <Footer/>
    }
}
