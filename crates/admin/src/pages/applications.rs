use leptos::prelude::*;
use serde::Deserialize;

use crate::api;
use crate::components::data_table::DataTable;
use crate::components::status_pill::StatusPill;

#[derive(Debug, Clone, Deserialize)]
struct Application {
    id: String,
    user_id: String,
    status: String,
    preferred_games: Vec<String>,
    message: Option<String>,
    created_at: String,
}

#[component]
pub fn ApplicationsPage() -> impl IntoView {
    let apps = LocalResource::new(|| async {
        api::get::<Vec<Application>>("/api/applications").await.ok()
    });

    view! {
        <h1>"Applications"</h1>
        {move || match apps.get().flatten() {
            None => view! { <p>"Loading..."</p> }.into_any(),
            Some(list) if list.is_empty() => view! { <p>"No applications."</p> }.into_any(),
            Some(list) => view! {
                <DataTable headers=vec!["User", "Games", "Status", "Message"]>
                    {list.into_iter().map(|a| {
                        let games = a.preferred_games.join(", ");
                        let msg = a.message.unwrap_or_else(|| "—".into());
                        view! {
                            <tr>
                                <td>{a.user_id}</td>
                                <td>{games}</td>
                                <td><StatusPill status=a.status/></td>
                                <td>{msg}</td>
                            </tr>
                        }
                    }).collect_view()}
                </DataTable>
            }.into_any(),
        }}
    }
}
