use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};

use scuffed_ui::components::button::{Button, ButtonVariant};
use scuffed_ui::components::toast::{use_toast, Toast};

use crate::api;
use crate::components::data_table::DataTable;
use crate::components::form_modal::FormModal;
use crate::components::forms::{CheckboxField, FormField, SelectField, TextAreaField};

// ─── Types ───

#[derive(Debug, Clone, Deserialize)]
struct Tournament {
    id: String,
    name: String,
    game_id: Option<String>,
    format: String,
    status: String,
    max_teams: Option<u32>,
    best_of: u32,
    swiss_rounds: Option<u32>,
    is_external: bool,
    is_open: bool,
    external_url: Option<String>,
    rules: Option<String>,
    description: Option<String>,
    starts_at: Option<String>,
    ends_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TournamentParticipant {
    id: String,
    #[allow(dead_code)]
    tournament_id: String,
    team_id: Option<String>,
    external_name: Option<String>,
    seed: Option<u32>,
    status: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TournamentMatch {
    id: String,
    #[allow(dead_code)]
    round_id: String,
    bracket_position: u32,
    participant_a_id: Option<String>,
    participant_b_id: Option<String>,
    score_a: Option<u32>,
    score_b: Option<u32>,
    winner_id: Option<String>,
    status: String,
    #[serde(default)]
    #[allow(dead_code)]
    replay_codes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct BracketData {
    tournament: Tournament,
    participants: Vec<TournamentParticipant>,
    #[allow(dead_code)]
    rounds: Vec<serde_json::Value>,
    matches: Vec<TournamentMatch>,
}

#[derive(Debug, Clone, Deserialize)]
struct Game {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Team {
    id: String,
    name: String,
}

#[derive(Serialize)]
struct CreateTournamentBody {
    name: String,
    game_id: Option<String>,
    format: String,
    max_teams: Option<u32>,
    best_of: u32,
    swiss_rounds: Option<u32>,
    is_external: bool,
    is_open: bool,
    external_url: Option<String>,
    rules: Option<String>,
    description: Option<String>,
    starts_at: Option<String>,
    ends_at: Option<String>,
}

#[derive(Serialize)]
struct UpdateTournamentBody {
    name: Option<String>,
    game_id: Option<Option<String>>,
    format: Option<String>,
    max_teams: Option<Option<u32>>,
    best_of: Option<u32>,
    swiss_rounds: Option<Option<u32>>,
    is_external: Option<bool>,
    is_open: Option<bool>,
    external_url: Option<Option<String>>,
    rules: Option<Option<String>>,
    description: Option<Option<String>>,
    starts_at: Option<Option<String>>,
    ends_at: Option<Option<String>>,
}

#[derive(Serialize)]
struct AddParticipantBody {
    team_id: Option<String>,
    external_name: Option<String>,
    seed: Option<u32>,
}

#[derive(Serialize)]
struct StatusTransitionBody {
    status: String,
}

#[derive(Serialize)]
struct ReportMatchBody {
    score_a: u32,
    score_b: u32,
    winner_id: String,
    notes: Option<String>,
    replay_codes: Option<Vec<String>>,
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

fn status_class(s: &str) -> String {
    format!("status-pill {s}")
}

fn status_label(s: &str) -> &str {
    match s {
        "draft" => "Draft",
        "registration" => "Registration",
        "in_progress" => "In Progress",
        "completed" => "Completed",
        "archived" => "Archived",
        _ => s,
    }
}

// ─── Component ───

#[component]
pub fn TournamentsPage() -> impl IntoView {
    let toast = use_toast();
    let refresh = RwSignal::new(0u32);
    let status_filter = RwSignal::new(String::new());

    let tournaments = LocalResource::new(move || {
        refresh.get();
        let sf = status_filter.get();
        async move {
            let url = if sf.is_empty() {
                "/api/tournaments".to_string()
            } else {
                format!("/api/tournaments?status={sf}")
            };
            api::get::<Vec<Tournament>>(&url).await.ok()
        }
    });

    let games = LocalResource::new(|| async { api::get::<Vec<Game>>("/api/games").await.ok() });
    let teams = LocalResource::new(|| async { api::get::<Vec<Team>>("/api/teams").await.ok() });

    // ── Tournament Form State ──
    let form_open = RwSignal::new(false);
    let form_editing_id = RwSignal::new(Option::<String>::None);
    let form_name = RwSignal::new(String::new());
    let form_game_id = RwSignal::new(String::new());
    let form_format = RwSignal::new("single_elim".to_string());
    let form_max_teams = RwSignal::new(String::new());
    let form_best_of = RwSignal::new("1".to_string());
    let form_swiss_rounds = RwSignal::new(String::new());
    let form_is_external = RwSignal::new(false);
    let form_is_open = RwSignal::new(false);
    let form_external_url = RwSignal::new(String::new());
    let form_rules = RwSignal::new(String::new());
    let form_description = RwSignal::new(String::new());
    let form_starts_at = RwSignal::new(String::new());
    let form_ends_at = RwSignal::new(String::new());
    let form_submitting = RwSignal::new(false);

    // ── Detail View State ──
    let detail_id = RwSignal::new(Option::<String>::None);
    let detail_refresh = RwSignal::new(0u32);

    let bracket_data = LocalResource::new(move || {
        detail_refresh.get();
        let id = detail_id.get();
        async move {
            match id {
                Some(id) => api::get::<BracketData>(&format!("/api/tournaments/{id}/bracket"))
                    .await
                    .ok(),
                None => None,
            }
        }
    });

    // ── Participant Form State ──
    let add_team_id = RwSignal::new(String::new());
    let add_external_name = RwSignal::new(String::new());
    let add_seed = RwSignal::new(String::new());

    // ── Match Report State ──
    let report_open = RwSignal::new(false);
    let report_match_id = RwSignal::new(String::new());
    let report_match_a_name = RwSignal::new(String::new());
    let report_match_b_name = RwSignal::new(String::new());
    let report_match_a_id = RwSignal::new(String::new());
    let report_match_b_id = RwSignal::new(String::new());
    let report_score_a = RwSignal::new("0".to_string());
    let report_score_b = RwSignal::new("0".to_string());
    let report_winner = RwSignal::new(String::new());
    let report_replay_codes = RwSignal::new(String::new());
    let report_submitting = RwSignal::new(false);

    // ── Handlers ──

    let open_create = move || {
        form_editing_id.set(None);
        form_name.set(String::new());
        form_game_id.set(String::new());
        form_format.set("single_elim".to_string());
        form_max_teams.set(String::new());
        form_best_of.set("1".to_string());
        form_swiss_rounds.set(String::new());
        form_is_external.set(false);
        form_is_open.set(false);
        form_external_url.set(String::new());
        form_rules.set(String::new());
        form_description.set(String::new());
        form_starts_at.set(String::new());
        form_ends_at.set(String::new());
        form_open.set(true);
    };

    let open_edit = move |t: &Tournament| {
        form_editing_id.set(Some(t.id.clone()));
        form_name.set(t.name.clone());
        form_game_id.set(t.game_id.clone().unwrap_or_default());
        form_format.set(t.format.clone());
        form_max_teams.set(t.max_teams.map(|m| m.to_string()).unwrap_or_default());
        form_best_of.set(t.best_of.to_string());
        form_swiss_rounds.set(t.swiss_rounds.map(|s| s.to_string()).unwrap_or_default());
        form_is_external.set(t.is_external);
        form_is_open.set(t.is_open);
        form_external_url.set(t.external_url.clone().unwrap_or_default());
        form_rules.set(t.rules.clone().unwrap_or_default());
        form_description.set(t.description.clone().unwrap_or_default());
        form_starts_at.set(t.starts_at.as_deref().map(|s| s.chars().take(16).collect()).unwrap_or_default());
        form_ends_at.set(t.ends_at.as_deref().map(|s| s.chars().take(16).collect()).unwrap_or_default());
        form_open.set(true);
    };

    let do_submit = move || {
        let editing_id = form_editing_id.get();
        let name = form_name.get();
        let game_id = form_game_id.get();
        let format = form_format.get();
        let max_teams: Option<u32> = form_max_teams.get().parse().ok();
        let best_of: u32 = form_best_of.get().parse().unwrap_or(1);
        let swiss_rounds: Option<u32> = form_swiss_rounds.get().parse().ok();
        let is_external = form_is_external.get();
        let is_open = form_is_open.get();
        let external_url = form_external_url.get();
        let rules = form_rules.get();
        let description = form_description.get();
        let starts_at = form_starts_at.get();
        let ends_at = form_ends_at.get();
        form_submitting.set(true);

        let to_iso = |s: String| -> Option<String> {
            if s.is_empty() {
                None
            } else if s.contains('Z') || s.contains('+') {
                Some(s)
            } else {
                Some(format!("{s}:00Z"))
            }
        };

        spawn_local(async move {
            let result = if let Some(id) = editing_id {
                let body = UpdateTournamentBody {
                    name: Some(name),
                    game_id: Some(if game_id.is_empty() { None } else { Some(game_id) }),
                    format: Some(format),
                    max_teams: Some(max_teams),
                    best_of: Some(best_of),
                    swiss_rounds: Some(swiss_rounds),
                    is_external: Some(is_external),
                    is_open: Some(is_open),
                    external_url: Some(if external_url.is_empty() { None } else { Some(external_url) }),
                    rules: Some(if rules.is_empty() { None } else { Some(rules) }),
                    description: Some(if description.is_empty() { None } else { Some(description) }),
                    starts_at: Some(to_iso(starts_at)),
                    ends_at: Some(to_iso(ends_at)),
                };
                api::put::<_, Tournament>(&format!("/api/tournaments/{id}"), &body)
                    .await
                    .map(|_| "Tournament updated")
            } else {
                let body = CreateTournamentBody {
                    name,
                    game_id: if game_id.is_empty() { None } else { Some(game_id) },
                    format,
                    max_teams,
                    best_of,
                    swiss_rounds,
                    is_external,
                    is_open,
                    external_url: if external_url.is_empty() { None } else { Some(external_url) },
                    rules: if rules.is_empty() { None } else { Some(rules) },
                    description: if description.is_empty() { None } else { Some(description) },
                    starts_at: to_iso(starts_at),
                    ends_at: to_iso(ends_at),
                };
                api::post::<_, Tournament>("/api/tournaments", &body)
                    .await
                    .map(|_| "Tournament created")
            };

            match result {
                Ok(msg) => {
                    toast.show(Toast::success(msg));
                    form_open.set(false);
                    refresh.update(|n| *n += 1);
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
            form_submitting.set(false);
        });
    };

    let do_add_participant = move || {
        let tid = detail_id.get().unwrap_or_default();
        let team_id = add_team_id.get();
        let ext_name = add_external_name.get();
        let seed: Option<u32> = add_seed.get().parse().ok();

        if team_id.is_empty() && ext_name.is_empty() {
            toast.show(Toast::warning("Select a team or enter external name"));
            return;
        }

        spawn_local(async move {
            let body = AddParticipantBody {
                team_id: if team_id.is_empty() { None } else { Some(team_id) },
                external_name: if ext_name.is_empty() { None } else { Some(ext_name) },
                seed,
            };
            match api::post::<_, TournamentParticipant>(
                &format!("/api/tournaments/{tid}/participants"),
                &body,
            )
            .await
            {
                Ok(_) => {
                    toast.show(Toast::success("Participant added"));
                    add_team_id.set(String::new());
                    add_external_name.set(String::new());
                    add_seed.set(String::new());
                    detail_refresh.update(|n| *n += 1);
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
        });
    };

    let do_remove_participant = move |pid: String| {
        let tid = detail_id.get().unwrap_or_default();
        spawn_local(async move {
            match api::delete(&format!("/api/tournaments/{tid}/participants/{pid}")).await {
                Ok(()) => {
                    toast.show(Toast::success("Participant removed"));
                    detail_refresh.update(|n| *n += 1);
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
        });
    };

    let do_generate_bracket = move || {
        let tid = detail_id.get().unwrap_or_default();
        spawn_local(async move {
            match api::post::<_, BracketData>(
                &format!("/api/tournaments/{tid}/generate-bracket"),
                &serde_json::json!({}),
            )
            .await
            {
                Ok(_) => {
                    toast.show(Toast::success("Bracket generated"));
                    detail_refresh.update(|n| *n += 1);
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
        });
    };

    let do_transition_status = move |new_status: String| {
        let tid = detail_id.get().unwrap_or_default();
        spawn_local(async move {
            let body = StatusTransitionBody { status: new_status.clone() };
            match api::patch::<_, Tournament>(&format!("/api/tournaments/{tid}/status"), &body).await
            {
                Ok(_) => {
                    toast.show(Toast::success(format!("Status changed to {new_status}")));
                    detail_refresh.update(|n| *n += 1);
                    refresh.update(|n| *n += 1);
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
        });
    };

    let do_report_match = move || {
        let tid = detail_id.get().unwrap_or_default();
        let mid = report_match_id.get();
        let score_a: u32 = report_score_a.get().parse().unwrap_or(0);
        let score_b: u32 = report_score_b.get().parse().unwrap_or(0);
        let winner_id = report_winner.get();

        if winner_id.is_empty() {
            toast.show(Toast::warning("Select a winner"));
            return;
        }

        let codes: Vec<String> = report_replay_codes.get()
            .split([',', '\n'])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        report_submitting.set(true);
        spawn_local(async move {
            let body = ReportMatchBody {
                score_a,
                score_b,
                winner_id,
                notes: None,
                replay_codes: if codes.is_empty() { None } else { Some(codes) },
            };
            match api::patch::<_, TournamentMatch>(
                &format!("/api/tournaments/{tid}/matches/{mid}/report"),
                &body,
            )
            .await
            {
                Ok(_) => {
                    toast.show(Toast::success("Match result reported"));
                    report_open.set(false);
                    detail_refresh.update(|n| *n += 1);
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
            report_submitting.set(false);
        });
    };

    let form_title = Signal::derive(move || {
        if form_editing_id.get().is_some() {
            "Edit Tournament".to_string()
        } else {
            "Create Tournament".to_string()
        }
    });

    // ── Main View ──

    view! {
        <h1>"Tournaments"</h1>

        // Back from detail view
        {move || detail_id.get().map(|_| view! {
            <div style="margin-bottom: 1rem;">
                <Button
                    variant=ButtonVariant::Ghost
                    on_click=Callback::new(move |_| detail_id.set(None))
                >
                    "\u{2190} Back to list"
                </Button>
            </div>
        })}

        // ── List View ──
        {move || if detail_id.get().is_none() {
            Some(view! {
                <div style="display: flex; align-items: center; gap: 1rem; margin-bottom: 1rem;">
                    <div>
                        <label style="color: var(--text-secondary); margin-right: 0.5rem;">"Status: "</label>
                        <select
                            style="background: var(--bg-surface); border: 1px solid var(--border); border-radius: 6px; padding: 0.4rem; color: var(--text-primary);"
                            on:change=move |ev| status_filter.set(event_target_value(&ev))
                        >
                            <option value="">"All"</option>
                            <option value="draft">"Draft"</option>
                            <option value="registration">"Registration"</option>
                            <option value="in_progress">"In Progress"</option>
                            <option value="completed">"Completed"</option>
                            <option value="archived">"Archived"</option>
                        </select>
                    </div>
                    <Button
                        variant=ButtonVariant::Primary
                        on_click=Callback::new(move |_| open_create())
                    >
                        "Add Tournament"
                    </Button>
                </div>

                {move || match tournaments.get().flatten() {
                    None => view! { <p>"Loading..."</p> }.into_any(),
                    Some(list) if list.is_empty() => view! { <p class="empty-state">"No tournaments."</p> }.into_any(),
                    Some(list) => view! {
                        <DataTable headers=vec!["Name", "Format", "Status", "Ext.", "Actions"]>
                            {list.into_iter().map(|t| {
                                let t_edit = t.clone();
                                let t_detail_id = t.id.clone();
                                let sc = status_class(&t.status);
                                let sl = status_label(&t.status).to_string();
                                let fl = format_label(&t.format).to_string();
                                let ext = if t.is_external { "Yes" } else { "\u{2014}" };

                                view! {
                                    <tr>
                                        <td>{t.name.clone()}</td>
                                        <td>{fl}</td>
                                        <td><span class=sc>{sl}</span></td>
                                        <td>{ext}</td>
                                        <td class="table-actions">
                                            <Button
                                                variant=ButtonVariant::Secondary
                                                on_click=Callback::new(move |_| open_edit(&t_edit))
                                            >
                                                "Edit"
                                            </Button>
                                            <Button
                                                variant=ButtonVariant::Ghost
                                                on_click=Callback::new(move |_| {
                                                    detail_id.set(Some(t_detail_id.clone()));
                                                    detail_refresh.update(|n| *n += 1);
                                                })
                                            >
                                                "Manage"
                                            </Button>
                                        </td>
                                    </tr>
                                }
                            }).collect_view()}
                        </DataTable>
                    }.into_any(),
                }}
            })
        } else {
            None
        }}

        // ── Detail View ──
        {move || {
            let data = bracket_data.get().flatten();
            if detail_id.get().is_none() || data.is_none() {
                return None;
            }
            let data = data.unwrap();
            let t = &data.tournament;
            let status = t.status.clone();
            let format = t.format.clone();

            // Build name lookup
            let team_list = teams.get().flatten().unwrap_or_default();
            let team_map: std::collections::HashMap<String, String> = team_list.iter()
                .map(|t| (t.id.clone(), t.name.clone()))
                .collect();
            let p_names: std::collections::HashMap<String, String> = data.participants.iter()
                .map(|p| {
                    let name = p.external_name.clone()
                        .or_else(|| p.team_id.as_ref().and_then(|tid| team_map.get(tid).cloned()))
                        .unwrap_or_else(|| "TBD".to_string());
                    (p.id.clone(), name)
                })
                .collect();

            // Status transition buttons
            let next_status = match status.as_str() {
                "draft" => Some(("registration", "Open Registration")),
                "registration" => Some(("in_progress", "Start Tournament")),
                "in_progress" => Some(("completed", "Mark Completed")),
                "completed" => Some(("archived", "Archive")),
                _ => None,
            };

            let ns = next_status.map(|(s, _)| s.to_string());
            let ns_label = next_status.map(|(_, l)| l.to_string());

            let can_generate = status == "draft" || status == "registration";
            let is_swiss = format == "swiss";
            let is_active = status == "in_progress";

            let status_cls = status_class(&status);
            let status_lbl = status_label(&status).to_string();
            let format_lbl = format_label(&format).to_string();

            Some(view! {
                <div style="margin-bottom: 1.5rem;">
                    <h2 style="font-family: var(--font-display); color: var(--text-bright); font-size: 1.4rem;">{t.name.clone()}</h2>
                    <div style="display: flex; gap: 1rem; align-items: center; margin-top: 0.5rem; flex-wrap: wrap;">
                        <span class=status_cls>{status_lbl}</span>
                        <span style="color: var(--text-muted); font-size: 0.85rem;">{format_lbl}</span>
                        <span style="color: var(--text-muted); font-size: 0.85rem;">{format!("Bo{}", t.best_of)}</span>
                        <span style="color: var(--text-muted); font-size: 0.85rem;">{format!("{} participants", data.participants.len())}</span>
                    </div>
                </div>

                // Status transition + generate
                <div style="display: flex; gap: 0.75rem; margin-bottom: 1.5rem; flex-wrap: wrap;">
                    {next_status.map(|_| {
                        let ns = ns.clone().unwrap();
                        let label = ns_label.clone().unwrap();
                        view! {
                            <Button
                                variant=ButtonVariant::Primary
                                on_click=Callback::new(move |_| do_transition_status(ns.clone()))
                            >
                                {label}
                            </Button>
                        }
                    })}
                    {can_generate.then(|| view! {
                        <Button
                            variant=ButtonVariant::Secondary
                            on_click=Callback::new(move |_| do_generate_bracket())
                        >
                            "Generate Bracket"
                        </Button>
                    })}
                    {(is_swiss && is_active).then(|| {
                        let tid = detail_id.get().unwrap_or_default();
                        view! {
                            <Button
                                variant=ButtonVariant::Secondary
                                on_click=Callback::new(move |_| {
                                    let tid = tid.clone();
                                    spawn_local(async move {
                                        match api::post::<_, serde_json::Value>(
                                            &format!("/api/tournaments/{tid}/next-round"),
                                            &serde_json::json!({}),
                                        ).await {
                                            Ok(_) => {
                                                toast.show(Toast::success("Next Swiss round generated"));
                                                detail_refresh.update(|n| *n += 1);
                                            }
                                            Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
                                        }
                                    });
                                })
                            >
                                "Next Swiss Round"
                            </Button>
                        }
                    })}
                </div>

                // Participants
                <h3 style="font-family: var(--font-display); color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em; font-size: 0.9rem; margin-bottom: 0.75rem;">"Participants"</h3>

                {if !data.participants.is_empty() {
                    let participants = data.participants.clone();
                    let p_names_tbl = p_names.clone();
                    view! {
                        <DataTable headers=vec!["#", "Name", "Status", "Actions"]>
                            {participants.into_iter().map(|p| {
                                let name = p_names_tbl.get(&p.id).cloned().unwrap_or("TBD".to_string());
                                let pid = p.id.clone();
                                view! {
                                    <tr>
                                        <td>{p.seed.map(|s| s.to_string()).unwrap_or("\u{2014}".to_string())}</td>
                                        <td>{name}</td>
                                        <td><span class=status_class(&p.status)>{p.status.clone()}</span></td>
                                        <td class="table-actions">
                                            <Button
                                                variant=ButtonVariant::Danger
                                                on_click=Callback::new(move |_| do_remove_participant(pid.clone()))
                                            >
                                                "Remove"
                                            </Button>
                                        </td>
                                    </tr>
                                }
                            }).collect_view()}
                        </DataTable>
                    }.into_any()
                } else {
                    view! { <p class="empty-state">"No participants yet."</p> }.into_any()
                }}

                // Add participant form
                <div style="display: flex; gap: 0.5rem; align-items: flex-end; margin-top: 1rem; margin-bottom: 2rem; flex-wrap: wrap;">
                    <div style="flex: 1; min-width: 150px;">
                        <label style="color: var(--text-secondary); font-size: 0.8rem;">"Org Team"</label>
                        <select
                            style="width: 100%; background: var(--bg-surface); border: 1px solid var(--border); border-radius: 6px; padding: 0.4rem; color: var(--text-primary);"
                            prop:value=move || add_team_id.get()
                            on:change=move |ev| add_team_id.set(event_target_value(&ev))
                        >
                            <option value="">"-- or external --"</option>
                            {move || teams.get().flatten().unwrap_or_default().into_iter().map(|t| {
                                view! { <option value={t.id.clone()}>{t.name}</option> }
                            }).collect_view()}
                        </select>
                    </div>
                    <div style="flex: 1; min-width: 150px;">
                        <label style="color: var(--text-secondary); font-size: 0.8rem;">"External Name"</label>
                        <input
                            class="form-input"
                            placeholder="External team name"
                            prop:value=move || add_external_name.get()
                            on:input=move |ev| add_external_name.set(event_target_value(&ev))
                        />
                    </div>
                    <div style="width: 80px;">
                        <label style="color: var(--text-secondary); font-size: 0.8rem;">"Seed"</label>
                        <input
                            class="form-input"
                            type="number"
                            placeholder="#"
                            prop:value=move || add_seed.get()
                            on:input=move |ev| add_seed.set(event_target_value(&ev))
                        />
                    </div>
                    <Button
                        variant=ButtonVariant::Primary
                        on_click=Callback::new(move |_| do_add_participant())
                    >
                        "Add"
                    </Button>
                </div>

                // Matches
                {if !data.matches.is_empty() {
                    let pending_matches: Vec<TournamentMatch> = data.matches.iter()
                        .filter(|m| m.status == "pending" || m.status == "scheduled" || m.status == "in_progress")
                        .filter(|m| m.participant_a_id.is_some() && m.participant_b_id.is_some())
                        .cloned()
                        .collect();

                    if !pending_matches.is_empty() {
                        let p_names_m = p_names.clone();
                        view! {
                            <h3 style="font-family: var(--font-display); color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em; font-size: 0.9rem; margin-bottom: 0.75rem;">"Pending Matches"</h3>
                            <DataTable headers=vec!["#", "Team A", "Team B", "Status", "Actions"]>
                                {pending_matches.into_iter().map(|m| {
                                    let a_name = m.participant_a_id.as_ref()
                                        .and_then(|id| p_names_m.get(id))
                                        .cloned()
                                        .unwrap_or("TBD".to_string());
                                    let b_name = m.participant_b_id.as_ref()
                                        .and_then(|id| p_names_m.get(id))
                                        .cloned()
                                        .unwrap_or("TBD".to_string());
                                    let mid = m.id.clone();
                                    let a_id = m.participant_a_id.clone().unwrap_or_default();
                                    let b_id = m.participant_b_id.clone().unwrap_or_default();
                                    let a_name2 = a_name.clone();
                                    let b_name2 = b_name.clone();

                                    view! {
                                        <tr>
                                            <td>{m.bracket_position}</td>
                                            <td>{a_name}</td>
                                            <td>{b_name}</td>
                                            <td><span class=status_class(&m.status)>{m.status.clone()}</span></td>
                                            <td class="table-actions">
                                                <Button
                                                    variant=ButtonVariant::Primary
                                                    on_click=Callback::new(move |_| {
                                                        report_match_id.set(mid.clone());
                                                        report_match_a_name.set(a_name2.clone());
                                                        report_match_b_name.set(b_name2.clone());
                                                        report_match_a_id.set(a_id.clone());
                                                        report_match_b_id.set(b_id.clone());
                                                        report_score_a.set("0".to_string());
                                                        report_score_b.set("0".to_string());
                                                        report_replay_codes.set(String::new());
                                                        report_winner.set(String::new());
                                                        report_open.set(true);
                                                    })
                                                >
                                                    "Report"
                                                </Button>
                                            </td>
                                        </tr>
                                    }
                                }).collect_view()}
                            </DataTable>
                        }.into_any()
                    } else {
                        view! { <p style="color: var(--text-muted); margin-top: 1rem;">"All matches have been played."</p> }.into_any()
                    }
                } else {
                    view! { <p style="color: var(--text-muted); margin-top: 1rem;">"Generate a bracket to see matches."</p> }.into_any()
                }}
            })
        }}

        // ── Tournament Form Modal ──
        <FormModal
            open=form_open
            on_close=Callback::new(move |_| form_open.set(false))
            title=form_title
            on_submit=Callback::new(move |_| do_submit())
            submitting=form_submitting
        >
            <FormField label="Name" value=form_name/>
            <div class="form-group">
                <label class="form-label">"Game"</label>
                <select
                    class="form-input"
                    prop:value=move || form_game_id.get()
                    on:change=move |ev| form_game_id.set(event_target_value(&ev))
                >
                    <option value="">"-- None --"</option>
                    {move || games.get().flatten().unwrap_or_default().into_iter().map(|g| {
                        view! { <option value={g.id.clone()}>{g.name}</option> }
                    }).collect_view()}
                </select>
            </div>
            <SelectField
                label="Format"
                value=form_format
                options=vec![
                    ("single_elim", "Single Elimination"),
                    ("double_elim", "Double Elimination"),
                    ("round_robin", "Round Robin"),
                    ("swiss", "Swiss"),
                ]
            />
            <FormField label="Max Teams" value=form_max_teams input_type="number"/>
            <FormField label="Best Of" value=form_best_of input_type="number"/>
            <FormField label="Swiss Rounds" value=form_swiss_rounds input_type="number"/>
            <CheckboxField label="External Tournament" value=form_is_external/>
            <CheckboxField label="Open Registration" value=form_is_open/>
            <FormField label="External URL" value=form_external_url/>
            <TextAreaField label="Description" value=form_description rows=3/>
            <TextAreaField label="Rules" value=form_rules rows=3/>
            <FormField label="Starts At" value=form_starts_at input_type="datetime-local"/>
            <FormField label="Ends At" value=form_ends_at input_type="datetime-local"/>
        </FormModal>

        // ── Report Match Modal ──
        <FormModal
            open=report_open
            on_close=Callback::new(move |_| report_open.set(false))
            title=Signal::derive(|| "Report Match Result".to_string())
            on_submit=Callback::new(move |_| do_report_match())
            submitting=report_submitting
        >
            <div style="margin-bottom: 1rem; color: var(--text-secondary);">
                {move || format!("{} vs {}", report_match_a_name.get(), report_match_b_name.get())}
            </div>
            <FormField label="Score A" value=report_score_a input_type="number"/>
            <FormField label="Score B" value=report_score_b input_type="number"/>
            <TextAreaField label="Replay Codes" value=report_replay_codes rows=3/>
            <p style="margin-top: -0.5rem; margin-bottom: 0.75rem; font-size: 0.75rem; color: var(--text-muted);">"One code per line, or comma-separated"</p>
            <div class="form-group">
                <label class="form-label">"Winner"</label>
                <select
                    class="form-input"
                    prop:value=move || report_winner.get()
                    on:change=move |ev| report_winner.set(event_target_value(&ev))
                >
                    <option value="">"Select winner..."</option>
                    <option value={move || report_match_a_id.get()}>
                        {move || report_match_a_name.get()}
                    </option>
                    <option value={move || report_match_b_id.get()}>
                        {move || report_match_b_name.get()}
                    </option>
                </select>
            </div>
        </FormModal>
    }
}
