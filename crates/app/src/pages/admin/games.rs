//! Admin › Games: the games teams and tournaments pick from.
//!
//! Officers see the list; only admins add or edit a game.

use dioxus::prelude::*;

use crate::components::ui::{
    EmptyCard, FieldError, PageHeader, PageShell, PageWidth, Pager, page_slice, show_list_top,
};
use crate::components::{
    ADMIN_PAGE_SIZE, DataTable, FormModal, RowActions, SaveState, Toast, admin_list_pending,
    list_cap_notice, use_toast,
};
use crate::hooks::{ModalController, use_api_list};
use crate::state::use_auth;
use scuffed_api_client::ApiClient;
use scuffed_types::{
    Game,
    api::{CreateGameRequest, UpdateGameRequest},
};

/// The list region: the pager's focus target after a page change.
const LIST_ID: &str = "admin-games-list";
const NEW_GAME_ID: &str = "games-new";
const NAME_ID: &str = "game-name";
const NAME_ERROR_ID: &str = "game-name-error";
const ABBR_ID: &str = "game-abbreviation";
const ABBR_HINT_ID: &str = "game-abbreviation-hint";

/// Id of a row's Edit button, from the game id. The dialog returns focus here.
fn edit_button_id(game_id: &str) -> String {
    format!("game-edit-{game_id}")
}

fn games_lead(is_admin: bool) -> &'static str {
    if is_admin {
        "The games your crew plays. Teams and tournaments pick a game from this list."
    } else {
        "The games your crew plays. Only admins can add or edit a game."
    }
}

fn empty_message(is_admin: bool) -> &'static str {
    if is_admin {
        "Add the games your crew plays, so teams and tournaments can pick one."
    } else {
        "No game is set up yet. Ask an admin to add the games your crew plays."
    }
}

/// Blank optional values read as the word None. A bare dash is not a value.
fn or_none(value: Option<&str>) -> String {
    match value.map(str::trim) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => "None".to_string(),
    }
}

#[component]
pub fn AdminGames() -> Element {
    let auth = use_auth();
    let is_admin = auth().is_admin();
    // Cursor-paginated envelope from GET /api/games (trust-p0).
    let mut games = use_api_list::<Game>("/api/games");
    let mut toast = use_toast();
    let mut page = use_signal(|| 1usize);

    // Modal state
    let mut modal = ModalController::<String>::new();
    let mut form_name = use_signal(String::new);
    let mut form_abbr = use_signal(String::new);
    let mut name_error = use_signal(String::new);
    let mut save_error = use_signal(|| None::<String>);
    let mut opener = use_signal(|| NEW_GAME_ID.to_string());

    let mut open_create = move || {
        form_name.set(String::new());
        form_abbr.set(String::new());
        name_error.set(String::new());
        save_error.set(None);
        opener.set(NEW_GAME_ID.to_string());
        modal.show_empty();
    };

    let mut open_edit = move |game: Game| {
        form_name.set(game.name);
        form_abbr.set(game.abbreviation.unwrap_or_default());
        name_error.set(String::new());
        save_error.set(None);
        opener.set(edit_button_id(&game.id));
        modal.show(game.id);
    };

    let on_close = move |_| {
        modal.close();
    };

    let on_submit = move |_| {
        let name = form_name().trim().to_string();
        if name.is_empty() {
            name_error.set("Enter a name for the game.".to_string());
            return;
        }
        name_error.set(String::new());
        save_error.set(None);
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
                    save_error.set(Some(
                        SaveState::failed("Could not save the game", &e).text(),
                    ));
                    toast.show(Toast::error_logged_elsewhere("Failed to save game", &e));
                }
            }
        });
    };

    let list = {
        let data = games.data.read();
        match data.as_ref().and_then(|d| d.as_ref()) {
            None => admin_list_pending(&games, "games"),
            Some(all) => {
                let total = all.len();
                let rows = page_slice(all, page(), ADMIN_PAGE_SIZE).to_vec();
                rsx! {
                    if all.is_empty() {
                        EmptyCard {
                            title: "No games yet",
                            message: empty_message(is_admin).to_string(),
                            actions: is_admin.then(|| rsx! {
                                button {
                                    r#type: "button",
                                    class: "btn is-primary",
                                    onclick: move |_| open_create(),
                                    "New game"
                                }
                            }),
                        }
                    } else {
                        DataTable {
                            headers: if is_admin {
                                vec!["Game", "Abbreviation", "Actions"]
                            } else {
                                vec!["Game", "Abbreviation"]
                            },
                            cards: true,
                            label: "Games",
                            for game in rows.iter() {
                                Fragment { key: "{game.id}", {
                                    let g = game.clone();
                                    let abbr = or_none(game.abbreviation.as_deref());
                                    let name = game.name.clone();
                                    rsx! {
                                        tr { key: "{game.id}",
                                            td { class: "cell-who", "{game.name}" }
                                            td { "data-label": "Abbreviation", "{abbr}" }
                                            if is_admin {
                                                td { class: "cell-actions",
                                                    RowActions {
                                                        button {
                                                            r#type: "button",
                                                            class: "btn is-ghost",
                                                            id: edit_button_id(&game.id),
                                                            aria_label: "Edit {name}",
                                                            onclick: move |_| open_edit(g.clone()),
                                                            "Edit"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                } }
                            }
                        }
                    }
                    // Always rendered: its count line is the list's live line.
                    Pager {
                        id: "games-pager",
                        page: page(),
                        total,
                        page_size: ADMIN_PAGE_SIZE,
                        noun: "games",
                        noun_singular: "game",
                        show_page_count: true,
                        on_change: move |next| {
                            page.set(next);
                            show_list_top(LIST_ID);
                        },
                    }
                }
            }
        }
    };

    let name_invalid = !name_error().is_empty();

    rsx! {
        PageShell { width: PageWidth::Wide, class: "admin-games-page",
            PageHeader {
                title: "Games",
                lead: games_lead(is_admin).to_string(),
                actions: is_admin.then(|| rsx! {
                    button {
                        r#type: "button",
                        class: "btn is-primary",
                        id: NEW_GAME_ID,
                        onclick: move |_| open_create(),
                        "New game"
                    }
                }),
            }

            section {
                id: LIST_ID,
                class: "admin-list pager-target",
                tabindex: "-1",
                aria_label: "Games",
                {list}
            }

            {list_cap_notice(&games, "games")}
        }

        FormModal {
            title: if modal.get_target().is_some() { "Edit game".to_string() } else { "New game".to_string() },
            open: modal.is_open(),
            submitting: modal.is_submitting(),
            on_close: on_close,
            on_submit: on_submit,
            submit_label: modal.get_target().is_none().then(|| "Create".to_string()),
            error: save_error(),
            restore_focus_id: Some(opener()),

            div { class: "form-field",
                label { class: "field-label", r#for: NAME_ID, "Name" }
                input {
                    class: "field-input",
                    id: NAME_ID,
                    r#type: "text",
                    value: "{form_name}",
                    aria_invalid: if name_invalid { "true" },
                    aria_describedby: NAME_ERROR_ID,
                    oninput: move |e| form_name.set(e.value()),
                }
                FieldError { id: NAME_ERROR_ID.to_string(), text: name_error() }
            }
            div { class: "form-field",
                label { class: "field-label", r#for: ABBR_ID, "Abbreviation (optional)" }
                input {
                    class: "field-input",
                    id: ABBR_ID,
                    r#type: "text",
                    placeholder: "OW",
                    aria_describedby: ABBR_HINT_ID,
                    value: "{form_abbr}",
                    oninput: move |e| form_abbr.set(e.value()),
                }
                p { class: "field-hint", id: ABBR_HINT_ID, "A short name for tight spaces, like OW." }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::admin_shared::test_page::render_admin_route;
    use scuffed_types::OrgRole;

    #[test]
    fn page_has_one_h1_and_a_labelled_wait_in_the_wide_shell() {
        let html = render_admin_route("/admin/games", Some(OrgRole::Admin));
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide admin-games-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Games</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(html.contains("id=\"games-new\""), "{html}");
        assert!(
            html.contains(
                "<section id=\"admin-games-list\" class=\"admin-list pager-target\" tabindex=\"-1\" aria-label=\"Games\">"
            ),
            "{html}"
        );
        assert!(
            html.contains("role=\"status\">Loading games...</p>"),
            "{html}"
        );
    }

    #[test]
    fn officers_get_no_new_button_and_are_told_why() {
        let html = render_admin_route("/admin/games", Some(OrgRole::Officer));
        assert!(!html.contains("id=\"games-new\""), "{html}");
        assert!(
            html.contains("Only admins can add or edit a game."),
            "{html}"
        );
    }

    #[test]
    fn dialog_fields_are_tied_to_labels_and_the_error_line_is_always_there() {
        fn view() -> Element {
            let name_error = String::new();
            rsx! {
                FormModal {
                    title: "New game".to_string(),
                    open: true,
                    submitting: false,
                    on_close: |_| {},
                    on_submit: |_| {},
                    div { class: "form-field",
                        label { class: "field-label", r#for: NAME_ID, "Name" }
                        input { class: "field-input", id: NAME_ID, aria_describedby: NAME_ERROR_ID }
                        FieldError { id: NAME_ERROR_ID.to_string(), text: name_error }
                    }
                }
            }
        }
        let html = crate::components::ui::test_support::render(view);
        assert!(html.contains("for=\"game-name\""), "{html}");
        assert!(html.contains("id=\"game-name\""), "{html}");
        assert!(html.contains("id=\"game-name-error\""), "{html}");
        assert!(html.contains("role=\"status\""), "{html}");
        // The real dialog uses the same ids.
        let src = include_str!("games.rs");
        let prod = src.split("mod tests").next().unwrap();
        assert!(prod.contains("r#for: NAME_ID"));
        assert!(prod.contains("r#for: ABBR_ID"));
        assert!(!prod.contains("style:"), "no inline styles");
        assert!(!prod.contains('\u{2014}'));
    }

    #[test]
    fn blank_values_read_none() {
        assert_eq!(or_none(None), "None");
        assert_eq!(or_none(Some("  ")), "None");
        assert_eq!(or_none(Some("OW")), "OW");
        assert_eq!(edit_button_id("g1"), "game-edit-g1");
    }
}
