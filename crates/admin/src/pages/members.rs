use leptos::prelude::*;
use serde::Deserialize;

use crate::api;
use crate::components::data_table::DataTable;
use crate::components::status_pill::RolePill;

#[derive(Debug, Clone, Deserialize)]
struct Member {
    id: String,
    display_name: String,
    org_role: String,
    user_id: String,
    is_active: bool,
}

#[component]
pub fn MembersPage() -> impl IntoView {
    let members =
        LocalResource::new(|| async { api::get::<Vec<Member>>("/api/members").await.ok() });

    view! {
        <h1>"Members"</h1>
        {move || match members.get().flatten() {
            None => view! { <p>"Loading..."</p> }.into_any(),
            Some(list) if list.is_empty() => view! { <p>"No members yet."</p> }.into_any(),
            Some(list) => view! {
                <DataTable headers=vec!["Name", "Role", "Status"]>
                    {list.into_iter().map(|m| {
                        let active = if m.is_active { "Active" } else { "Inactive" };
                        view! {
                            <tr>
                                <td>{m.display_name}</td>
                                <td><RolePill role=m.org_role/></td>
                                <td>{active}</td>
                            </tr>
                        }
                    }).collect_view()}
                </DataTable>
            }.into_any(),
        }}
    }
}
