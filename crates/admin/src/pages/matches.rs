use leptos::prelude::*;
use serde::Deserialize;

use crate::api;
use crate::components::data_table::DataTable;

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
    played_at: String,
}

#[component]
pub fn MatchesPage() -> impl IntoView {
    let teams = LocalResource::new(|| async { api::get::<Vec<Team>>("/api/teams").await.ok() });

    let selected_team = RwSignal::new(String::new());

    let matches = LocalResource::new(move || {
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

    view! {
        <h1>"Matches"</h1>
        <div style="margin-bottom: 1rem;">
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
        {move || {
            if selected_team.get().is_empty() {
                return view! { <p style="color: var(--text-muted);">"Select a team to view matches."</p> }.into_any();
            }
            match matches.get().flatten() {
                None => view! { <p>"Loading..."</p> }.into_any(),
                Some(list) if list.is_empty() => view! { <p>"No matches recorded."</p> }.into_any(),
                Some(list) => view! {
                    <DataTable headers=vec!["Opponent", "Score", "Date"]>
                        {list.into_iter().map(|m| {
                            let score = format!("{} - {}", m.score_us, m.score_them);
                            view! {
                                <tr>
                                    <td>{m.opponent}</td>
                                    <td>{score}</td>
                                    <td>{m.played_at}</td>
                                </tr>
                            }
                        }).collect_view()}
                    </DataTable>
                }.into_any(),
            }
        }}
    }
}
