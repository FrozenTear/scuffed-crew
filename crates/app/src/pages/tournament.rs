use dioxus::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;

use scuffed_api_client::ApiClient;
use crate::components::bracket::{BracketView, BracketMatch, BracketRound, SwissStanding, BRACKET_STYLES};

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

const PAGE_CSS: &str = r#"
    .tournament-detail-page {
        padding: 3rem 2rem;
        max-width: 1100px;
        margin: 0 auto;
    }
    .tournament-header {
        margin-bottom: 2.5rem;
    }
    .tournament-header h1 {
        font-family: 'Bebas Neue', sans-serif;
        font-size: 2.5rem;
        color: var(--text-bright);
        letter-spacing: 3px;
        margin: 0 0 0.75rem;
    }
    .tournament-meta {
        display: flex;
        gap: 0.75rem;
        flex-wrap: wrap;
        align-items: center;
        font-size: 0.8rem;
        color: var(--text-muted);
        margin-bottom: 1rem;
    }
    .tournament-card-status {
        display: inline-block;
        font-size: 0.65rem;
        padding: 0.15rem 0.6rem;
        border-radius: 999px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
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
    .tournament-description {
        color: var(--text-secondary);
        font-size: 0.9rem;
        line-height: 1.7;
        margin: 0 0 1rem;
    }
    .tournament-external-link {
        color: var(--accent);
        font-size: 0.85rem;
        text-decoration: none;
        word-break: break-all;
    }
    .tournament-external-link:hover {
        text-decoration: underline;
    }
    .tournament-section-title {
        font-family: 'Rajdhani', sans-serif;
        font-weight: 700;
        font-size: 1.2rem;
        color: var(--text-bright);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        margin: 2rem 0 1rem;
        padding-top: 1.5rem;
        border-top: 1px solid var(--border);
    }
    .tournament-loading {
        color: var(--text-muted);
        text-align: center;
        padding: 3rem 0;
    }
    .tournament-no-bracket {
        color: var(--text-muted);
        text-align: center;
        padding: 2rem 0;
    }
"#;

#[component]
pub fn Tournament(id: String) -> Element {
    let id_for_bracket = id.clone();
    let id_for_standings = id.clone();

    let bracket_data = use_resource(move || {
        let id = id_for_bracket.clone();
        async move {
            if id.is_empty() {
                return None;
            }
            ApiClient::web()
                .fetch::<BracketData>(&format!("/api/tournaments/{id}/bracket"))
                .await
                .ok()
        }
    });

    let teams = use_resource(|| async {
        ApiClient::web().fetch::<Vec<Team>>("/api/teams").await.ok()
    });

    let standings = use_resource(move || {
        let id = id_for_standings.clone();
        async move {
            ApiClient::web()
                .fetch::<Vec<SwissStanding>>(&format!("/api/tournaments/{id}/standings"))
                .await
                .ok()
        }
    });

    rsx! {
        style { {PAGE_CSS} }
        style { {BRACKET_STYLES} }

        main { class: "tournament-detail-page",
            {
                let team_map: HashMap<String, String> = teams.read()
                    .as_ref()
                    .and_then(|t| t.as_ref())
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|t| (t.id, t.name))
                    .collect();

                let data = bracket_data.read();
                let data = data.as_ref().and_then(|d| d.as_ref());

                match data {
                    None => rsx! { p { class: "tournament-loading", "Loading..." } },
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

                        let swiss = standings.read()
                            .as_ref()
                            .and_then(|s| s.as_ref())
                            .cloned()
                            .unwrap_or_default();

                        let name = t.name.clone();
                        let bo = t.best_of;
                        let desc = t.description.clone();
                        let rules = t.rules.clone();
                        let ext_url = t.external_url.clone();
                        let is_ext = t.is_external;
                        let starts = t.starts_at.clone();
                        let format_str = t.format.clone();
                        let has_matches = !data.matches.is_empty();
                        let rounds = data.rounds.clone();
                        let matches = data.matches.clone();

                        rsx! {
                            div { class: "tournament-header",
                                h1 { "{name}" }
                                div { class: "tournament-meta",
                                    span { "{format_text}" }
                                    if bo > 1 {
                                        span { "Bo{bo}" }
                                    }
                                    span { class: "{status_class}", "{status_text}" }
                                    if let Some(d) = &starts {
                                        span { "{d.chars().take(10).collect::<String>()}" }
                                    }
                                    if is_ext {
                                        span { "External" }
                                    }
                                }
                                if let Some(d) = &desc {
                                    p { class: "tournament-description", "{d}" }
                                }
                                if let Some(url) = &ext_url {
                                    a {
                                        href: "{url}",
                                        target: "_blank",
                                        class: "tournament-external-link",
                                        "{url}"
                                    }
                                }
                            }

                            if has_matches {
                                div { class: "tournament-section-title", "Bracket" }
                                BracketView {
                                    format: format_str,
                                    rounds: rounds,
                                    matches: matches,
                                    participant_names: pnames,
                                    participant_ids: pids,
                                    swiss_standings: swiss,
                                }
                            } else {
                                p { class: "tournament-no-bracket", "Bracket not yet generated." }
                            }

                            if let Some(r) = &rules {
                                div { class: "tournament-section-title", "Rules" }
                                p { class: "tournament-description", "{r}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
