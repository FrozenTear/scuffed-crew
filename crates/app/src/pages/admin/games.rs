use dioxus::prelude::*;

use crate::components::{DataTable, FormModal, Toast, admin_pending, use_toast};
use crate::hooks::{ModalController, use_api};
use scuffed_api_client::ApiClient;
use scuffed_types::{
    Game,
    api::{CreateGameRequest, UpdateGameRequest},
};

#[component]
pub fn AdminGames() -> Element {
    let mut games = use_api::<Vec<Game>>("/api/games");
    let mut toast = use_toast();

    // Modal state
    let mut modal = ModalController::<String>::new();
    let mut form_name = use_signal(String::new);
    let mut form_abbr = use_signal(String::new);

    let open_create = move |_| {
        form_name.set(String::new());
        form_abbr.set(String::new());
        modal.show_empty();
    };

    let mut open_edit = move |game: Game| {
        form_name.set(game.name);
        form_abbr.set(game.abbreviation.unwrap_or_default());
        modal.show(game.id);
    };

    let on_close = move |_| {
        modal.close();
    };

    let on_submit = move |_| {
        let name = form_name().trim().to_string();
        if name.is_empty() {
            return;
        }
        let abbr_raw = form_abbr().trim().to_string();
        let abbr = if abbr_raw.is_empty() {
            None
        } else {
            Some(abbr_raw)
        };
        let edit_id = modal.get_target();

        modal.start_submit();
        spawn(async move {
            let client = ApiClient::web();
            let result = if let Some(id) = edit_id {
                let body = UpdateGameRequest {
                    name: Some(name),
                    abbreviation: Some(abbr),
                };
                client
                    .put_json::<_, Game>(&format!("/api/games/{id}"), &body)
                    .await
            } else {
                let body = CreateGameRequest {
                    name,
                    abbreviation: abbr,
                };
                client.post_json::<_, Game>("/api/games", &body).await
            };

            modal.end_submit();
            match result {
                Ok(_) => {
                    toast.show(Toast::success("Game saved."));
                    modal.close();
                    games.refresh += 1;
                }
                Err(e) => {
                    toast.show(Toast::error(format!("Failed to save game: {e}")));
                }
            }
        });
    };

    rsx! {

        div { class: "admin-toolbar",
            h1 { "Games" }
            button { class: "btn-add", onclick: open_create, "+ Add Game" }
        }

        {
            let data = games.data.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => admin_pending(&games, "games"),
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
            title: if modal.get_target().is_some() { "Edit Game".to_string() } else { "Add Game".to_string() },
            open: modal.is_open(),
            submitting: modal.is_submitting(),
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
