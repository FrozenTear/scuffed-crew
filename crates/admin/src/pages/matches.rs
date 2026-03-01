use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};

use scuffed_ui::components::button::{Button, ButtonVariant};
use scuffed_ui::components::toast::{use_toast, Toast};

use crate::api;
use crate::components::data_table::DataTable;
use crate::components::form_modal::FormModal;
use crate::components::forms::{FormField, TextAreaField};

#[derive(Debug, Clone, Deserialize)]
struct Team {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct MatchResult {
    id: String,
    team_id: String,
    opponent: String,
    score_us: u32,
    score_them: u32,
    map_name: Option<String>,
    game_mode: Option<String>,
    played_at: String,
    notes: Option<String>,
}

#[derive(Serialize)]
struct RecordMatchBody {
    team_id: String,
    opponent: String,
    score_us: u32,
    score_them: u32,
    map_name: Option<String>,
    game_mode: Option<String>,
    played_at: String,
    notes: Option<String>,
}

#[derive(Serialize)]
struct UpdateMatchBody {
    opponent: Option<String>,
    score_us: Option<u32>,
    score_them: Option<u32>,
    map_name: Option<Option<String>>,
    game_mode: Option<Option<String>>,
    notes: Option<Option<String>>,
}

#[component]
pub fn MatchesPage() -> impl IntoView {
    let toast = use_toast();
    let teams = LocalResource::new(|| async { api::get::<Vec<Team>>("/api/teams").await.ok() });

    let selected_team = RwSignal::new(String::new());
    let refresh = RwSignal::new(0u32);

    let matches = LocalResource::new(move || {
        refresh.get();
        let tid = selected_team.get();
        async move {
            if tid.is_empty() {
                return None;
            }
            api::get::<Vec<MatchResult>>(&format!("/api/teams/{tid}/matches"))
                .await
                .ok()
        }
    });

    // Form modal state
    let form_open = RwSignal::new(false);
    let form_editing_id = RwSignal::new(Option::<String>::None);
    let form_opponent = RwSignal::new(String::new());
    let form_score_us = RwSignal::new(String::new());
    let form_score_them = RwSignal::new(String::new());
    let form_map = RwSignal::new(String::new());
    let form_mode = RwSignal::new(String::new());
    let form_played_at = RwSignal::new(String::new());
    let form_notes = RwSignal::new(String::new());
    let form_submitting = RwSignal::new(false);

    let open_record = move || {
        form_editing_id.set(None);
        form_opponent.set(String::new());
        form_score_us.set("0".to_string());
        form_score_them.set("0".to_string());
        form_map.set(String::new());
        form_mode.set(String::new());
        form_played_at.set(String::new());
        form_notes.set(String::new());
        form_open.set(true);
    };

    let open_edit = move |m: &MatchResult| {
        form_editing_id.set(Some(m.id.clone()));
        form_opponent.set(m.opponent.clone());
        form_score_us.set(m.score_us.to_string());
        form_score_them.set(m.score_them.to_string());
        form_map.set(m.map_name.clone().unwrap_or_default());
        form_mode.set(m.game_mode.clone().unwrap_or_default());
        // played_at comes as ISO string, convert for datetime-local input
        form_played_at.set(m.played_at.chars().take(16).collect());
        form_notes.set(m.notes.clone().unwrap_or_default());
        form_open.set(true);
    };

    let do_submit = move || {
        let editing_id = form_editing_id.get();
        let opponent = form_opponent.get();
        let score_us: u32 = form_score_us.get().parse().unwrap_or(0);
        let score_them: u32 = form_score_them.get().parse().unwrap_or(0);
        let map = form_map.get();
        let mode = form_mode.get();
        let played_at = form_played_at.get();
        let notes = form_notes.get();
        form_submitting.set(true);

        // For new matches, append :00Z to make ISO 8601
        let played_at_iso = if played_at.contains('Z') || played_at.contains('+') {
            played_at
        } else {
            format!("{played_at}:00Z")
        };

        spawn_local(async move {
            let result = if let Some(id) = editing_id {
                let body = UpdateMatchBody {
                    opponent: Some(opponent),
                    score_us: Some(score_us),
                    score_them: Some(score_them),
                    map_name: Some(if map.is_empty() { None } else { Some(map) }),
                    game_mode: Some(if mode.is_empty() { None } else { Some(mode) }),
                    notes: Some(if notes.is_empty() { None } else { Some(notes) }),
                };
                api::put::<_, MatchResult>(&format!("/api/matches/{id}"), &body)
                    .await
                    .map(|_| "Match updated")
            } else {
                let tid = selected_team.get();
                let body = RecordMatchBody {
                    team_id: tid,
                    opponent,
                    score_us,
                    score_them,
                    map_name: if map.is_empty() { None } else { Some(map) },
                    game_mode: if mode.is_empty() { None } else { Some(mode) },
                    played_at: played_at_iso,
                    notes: if notes.is_empty() { None } else { Some(notes) },
                };
                api::post::<_, MatchResult>("/api/matches", &body)
                    .await
                    .map(|_| "Match recorded")
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

    let modal_title = Signal::derive(move || {
        if form_editing_id.get().is_some() {
            "Edit Match".to_string()
        } else {
            "Record Match".to_string()
        }
    });

    let has_team = move || !selected_team.get().is_empty();

    view! {
        <h1>"Matches"</h1>
        <div style="display: flex; align-items: center; gap: 1rem; margin-bottom: 1rem;">
            <div>
                <label style="color: var(--text-secondary); margin-right: 0.5rem;">"Team: "</label>
                <select
                    style="background: var(--bg-surface); border: 1px solid var(--border); border-radius: 6px; padding: 0.4rem; color: var(--text-primary);"
                    on:change=move |ev| selected_team.set(event_target_value(&ev))
                >
                    <option value="">"Select a team..."</option>
                    {move || teams.get().flatten().unwrap_or_default().into_iter().map(|t| {
                        view! { <option value={t.id.clone()}>{t.name}</option> }
                    }).collect_view()}
                </select>
            </div>
            {move || if has_team() {
                Some(view! {
                    <Button
                        variant=ButtonVariant::Primary
                        on_click=Callback::new(move |_| open_record())
                    >
                        "Record Match"
                    </Button>
                })
            } else {
                None
            }}
        </div>
        {move || {
            if selected_team.get().is_empty() {
                return view! { <p style="color: var(--text-muted);">"Select a team to view matches."</p> }.into_any();
            }
            match matches.get().flatten() {
                None => view! { <p>"Loading..."</p> }.into_any(),
                Some(list) if list.is_empty() => view! { <p>"No matches recorded."</p> }.into_any(),
                Some(list) => view! {
                    <DataTable headers=vec!["Opponent", "Score", "Map", "Date", "Actions"]>
                        {list.into_iter().map(|m| {
                            let score = format!("{} - {}", m.score_us, m.score_them);
                            let map = m.map_name.clone().unwrap_or_else(|| "\u{2014}".into());
                            let m_edit = m.clone();
                            view! {
                                <tr>
                                    <td>{m.opponent.clone()}</td>
                                    <td>{score}</td>
                                    <td>{map}</td>
                                    <td>{m.played_at.chars().take(10).collect::<String>()}</td>
                                    <td class="table-actions">
                                        <Button
                                            variant=ButtonVariant::Secondary
                                            on_click=Callback::new(move |_| open_edit(&m_edit))
                                        >
                                            "Edit"
                                        </Button>
                                    </td>
                                </tr>
                            }
                        }).collect_view()}
                    </DataTable>
                }.into_any(),
            }
        }}

        <FormModal
            open=form_open
            on_close=Callback::new(move |_| form_open.set(false))
            title=modal_title
            on_submit=Callback::new(move |_| do_submit())
            submitting=form_submitting
        >
            <FormField label="Opponent" value=form_opponent/>
            <FormField label="Our Score" value=form_score_us input_type="number"/>
            <FormField label="Their Score" value=form_score_them input_type="number"/>
            <FormField label="Map Name" value=form_map/>
            <FormField label="Game Mode" value=form_mode/>
            <FormField label="Played At" value=form_played_at input_type="datetime-local"/>
            <TextAreaField label="Notes" value=form_notes rows=3/>
        </FormModal>
    }
}
