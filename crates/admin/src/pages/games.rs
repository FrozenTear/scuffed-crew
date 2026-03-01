use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};

use scuffed_ui::components::button::{Button, ButtonVariant};
use scuffed_ui::components::toast::{use_toast, Toast};

use crate::api;
use crate::components::data_table::DataTable;
use crate::components::form_modal::FormModal;
use crate::components::forms::FormField;

#[derive(Debug, Clone, Deserialize)]
struct Game {
    id: String,
    name: String,
    abbreviation: Option<String>,
    #[allow(dead_code)]
    is_active: bool,
}

#[derive(Serialize)]
struct CreateGameBody {
    name: String,
    abbreviation: Option<String>,
}

#[derive(Serialize)]
struct UpdateGameBody {
    name: Option<String>,
    abbreviation: Option<Option<String>>,
}

#[component]
pub fn GamesPage() -> impl IntoView {
    let refresh = RwSignal::new(0u32);
    let toast = use_toast();

    let games = LocalResource::new(move || {
        refresh.get();
        async { api::get::<Vec<Game>>("/api/games").await.ok() }
    });

    // Form state
    let form_open = RwSignal::new(false);
    let form_editing_id = RwSignal::new(Option::<String>::None);
    let form_name = RwSignal::new(String::new());
    let form_abbr = RwSignal::new(String::new());
    let form_submitting = RwSignal::new(false);

    let open_create = move || {
        form_editing_id.set(None);
        form_name.set(String::new());
        form_abbr.set(String::new());
        form_open.set(true);
    };

    let open_edit = move |g: &Game| {
        form_editing_id.set(Some(g.id.clone()));
        form_name.set(g.name.clone());
        form_abbr.set(g.abbreviation.clone().unwrap_or_default());
        form_open.set(true);
    };

    let do_submit = move || {
        let editing_id = form_editing_id.get();
        let name = form_name.get();
        let abbr = form_abbr.get();
        form_submitting.set(true);

        spawn_local(async move {
            let result = if let Some(id) = editing_id {
                let body = UpdateGameBody {
                    name: Some(name),
                    abbreviation: Some(if abbr.is_empty() { None } else { Some(abbr) }),
                };
                api::put::<_, Game>(&format!("/api/games/{id}"), &body)
                    .await
                    .map(|_| "Game updated")
            } else {
                let body = CreateGameBody {
                    name,
                    abbreviation: if abbr.is_empty() { None } else { Some(abbr) },
                };
                api::post::<_, Game>("/api/games", &body)
                    .await
                    .map(|_| "Game created")
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

    let form_title = Signal::derive(move || {
        if form_editing_id.get().is_some() {
            "Edit Game".to_string()
        } else {
            "Add Game".to_string()
        }
    });

    view! {
        <h1>"Games"</h1>
        <div class="page-actions">
            <Button
                variant=ButtonVariant::Primary
                on_click=Callback::new(move |_| open_create())
            >
                "Add Game"
            </Button>
        </div>
        {move || match games.get().flatten() {
            None => view! { <p class="empty-state">"Loading..."</p> }.into_any(),
            Some(list) if list.is_empty() => view! { <p class="empty-state">"No games yet. Add one to get started."</p> }.into_any(),
            Some(list) => view! {
                <DataTable headers=vec!["Name", "Abbreviation", "Actions"]>
                    {list.into_iter().map(|g| {
                        let g_edit = g.clone();
                        view! {
                            <tr>
                                <td>{g.name.clone()}</td>
                                <td>{g.abbreviation.clone().unwrap_or_else(|| "\u{2014}".into())}</td>
                                <td class="table-actions">
                                    <Button
                                        variant=ButtonVariant::Secondary
                                        on_click=Callback::new(move |_| open_edit(&g_edit))
                                    >
                                        "Edit"
                                    </Button>
                                </td>
                            </tr>
                        }
                    }).collect_view()}
                </DataTable>
            }.into_any(),
        }}

        <FormModal
            open=form_open
            on_close=Callback::new(move |_| form_open.set(false))
            title=form_title
            on_submit=Callback::new(move |_| do_submit())
            submitting=form_submitting
        >
            <FormField label="Name" value=form_name placeholder="e.g. Overwatch 2"/>
            <FormField label="Abbreviation" value=form_abbr placeholder="e.g. OW2"/>
        </FormModal>
    }
}
