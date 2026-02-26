use leptos::prelude::*;
use serde::Deserialize;

use crate::api;
use crate::components::data_table::DataTable;

#[derive(Debug, Clone, Deserialize)]
struct Team {
    id: String,
    name: String,
    game: String,
    division: Option<String>,
    is_active: bool,
}

#[component]
pub fn TeamsPage() -> impl IntoView {
    let teams = LocalResource::new(|| async { api::get::<Vec<Team>>("/api/teams").await.ok() });

    view! {
        <h1>"Teams"</h1>
        {move || match teams.get().flatten() {
            None => view! { <p>"Loading..."</p> }.into_any(),
            Some(list) if list.is_empty() => view! { <p>"No teams yet."</p> }.into_any(),
            Some(list) => view! {
                <DataTable headers=vec!["Name", "Game", "Division", "Status"]>
                    {list.into_iter().map(|t| {
                        let active = if t.is_active { "Active" } else { "Inactive" };
                        view! {
                            <tr>
                                <td>{t.name}</td>
                                <td>{t.game}</td>
                                <td>{t.division.unwrap_or_else(|| "—".into())}</td>
                                <td>{active}</td>
                            </tr>
                        }
                    }).collect_view()}
                </DataTable>
            }.into_any(),
        }}
    }
}
