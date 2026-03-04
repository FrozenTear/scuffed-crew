use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use scuffed_api_client::ApiClient;
use crate::components::{DataTable, FormModal, Toast, use_toast, ADMIN_SHARED_CSS};

#[derive(Debug, Clone, Deserialize)]
struct Game {
    id: String,
    name: String,
    abbreviation: Option<String>,
}

#[derive(Serialize)]
struct CreateGame {
    name: String,
    abbreviation: Option<String>,
}

#[derive(Serialize)]
struct UpdateGame {
    name: Option<String>,
    abbreviation: Option<Option<String>>,
}

#[component]
pub fn AdminGames() -> Element {
    let mut refresh = use_signal(|| 0u64);
    let mut toast = use_toast();

    // Modal state
    let mut modal_open = use_signal(|| false);
    let mut submitting = use_signal(|| false);
    let mut editing_id: Signal<Option<String>> = use_signal(|| None);
    let mut form_name = use_signal(String::new);
    let mut form_abbr = use_signal(String::new);

    let games = use_resource(move || async move {
        let _ = refresh();
        ApiClient::web().fetch::<Vec<Game>>("/api/games").await.ok()
    });

    let open_create = move |_| {
        editing_id.set(None);
        form_name.set(String::new());
        form_abbr.set(String::new());
        modal_open.set(true);
    };

    let mut open_edit = move |game: Game| {
        editing_id.set(Some(game.id));
        form_name.set(game.name);
        form_abbr.set(game.abbreviation.unwrap_or_default());
        modal_open.set(true);
    };

    let on_close = move |_| {
        modal_open.set(false);
    };

    let on_submit = move |_| {
        let name = form_name().trim().to_string();
        if name.is_empty() {
            return;
        }
        let abbr_raw = form_abbr().trim().to_string();
        let abbr = if abbr_raw.is_empty() { None } else { Some(abbr_raw) };
        let edit_id = editing_id();

        submitting.set(true);
        spawn(async move {
            let client = ApiClient::web();
            let result = if let Some(id) = edit_id {
                let body = UpdateGame {
                    name: Some(name),
                    abbreviation: Some(abbr),
                };
                client.put_json::<_, Game>(&format!("/api/games/{id}"), &body).await
            } else {
                let body = CreateGame { name, abbreviation: abbr };
                client.post_json::<_, Game>("/api/games", &body).await
            };

            submitting.set(false);
            match result {
                Ok(_) => {
                    toast.show(Toast::success("Game saved."));
                    modal_open.set(false);
                    refresh += 1;
                }
                Err(e) => {
                    toast.show(Toast::error(format!("Failed to save game: {e}")));
                }
            }
        });
    };

    rsx! {
        style { {ADMIN_SHARED_CSS} }

        div { class: "admin-toolbar",
            h1 { "Games" }
            button { class: "btn-add", onclick: open_create, "+ Add Game" }
        }

        {
            let data = games.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => rsx! { p { class: "admin-loading", "Loading..." } },
                Some(list) if list.is_empty() => rsx! {
                    p { class: "empty-state", "No games configured yet." }
                },
                Some(list) => rsx! {
                    DataTable { headers: vec!["Name", "Abbreviation", "Actions"],
                        for game in list.iter() {
                            {
                                let g = game.clone();
                                let abbr_display = game.abbreviation.clone().unwrap_or_else(|| "—".into());
                                rsx! {
                                    tr { key: "{game.id}",
                                        td { "{game.name}" }
                                        td { "{abbr_display}" }
                                        td {
                                            div { class: "row-actions",
                                                button {
                                                    class: "row-btn",
                                                    onclick: move |_| open_edit(g.clone()),
                                                    "Edit"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }
        }

        FormModal {
            title: if editing_id().is_some() { "Edit Game".to_string() } else { "Add Game".to_string() },
            open: modal_open(),
            submitting: submitting(),
            on_close: on_close,
            on_submit: on_submit,

            div { class: "form-field",
                label { class: "form-label", "Name" }
                input {
                    class: "form-input",
                    r#type: "text",
                    value: "{form_name}",
                    oninput: move |e| form_name.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Abbreviation" }
                input {
                    class: "form-input",
                    r#type: "text",
                    placeholder: "e.g. OW2",
                    value: "{form_abbr}",
                    oninput: move |e| form_abbr.set(e.value()),
                }
            }
        }
    }
}
