//! Admin › Seasons — define the windows that "per season" stats are cut by.
//!
//! A season is a half-open `[starts_at, ends_at)` range over `played_at`, in
//! UTC. Games are never tagged with a season; deleting or editing one only
//! changes how existing matches are grouped.

use chrono::{DateTime, NaiveDateTime, Utc};
use dioxus::prelude::*;

use crate::components::{
    AccessDenied, ConfirmDialog, DataTable, FormModal, Toast, admin_pending, use_toast,
};
use crate::hooks::{ModalController, use_api};
use crate::state::use_auth;
use scuffed_api_client::ApiClient;
use scuffed_types::{
    Season,
    api::{CreateSeasonRequest, UpdateSeasonRequest},
};

const INPUT_FMT: &str = "%Y-%m-%dT%H:%M";

fn to_input(dt: &DateTime<Utc>) -> String {
    dt.format(INPUT_FMT).to_string()
}

/// Parse a `datetime-local` value as UTC. Browsers may include seconds.
fn parse_input(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    NaiveDateTime::parse_from_str(s, INPUT_FMT)
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
        .ok()
        .map(|n| n.and_utc())
}

fn fmt_display(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M").to_string()
}

#[component]
pub fn AdminSeasons() -> Element {
    let auth = use_auth();
    let mut seasons = use_api::<Vec<Season>>("/api/admin/seasons");
    let mut toast = use_toast();

    let mut modal = ModalController::<String>::new();
    let mut delete_modal = ModalController::<Season>::new();
    let mut form_name = use_signal(String::new);
    let mut form_starts = use_signal(String::new);
    let mut form_ends = use_signal(String::new);
    let mut form_current = use_signal(|| false);

    let open_create = move |_| {
        form_name.set(String::new());
        form_starts.set(String::new());
        form_ends.set(String::new());
        form_current.set(false);
        modal.show_empty();
    };

    let mut open_edit = move |s: Season| {
        form_name.set(s.name);
        form_starts.set(to_input(&s.starts_at));
        form_ends.set(to_input(&s.ends_at));
        form_current.set(s.is_current);
        modal.show(s.id);
    };

    let mut open_delete = move |s: Season| {
        delete_modal.show(s);
    };

    let on_close = move |_| {
        modal.close();
    };

    let on_submit = move |_| {
        let name = form_name().trim().to_string();
        if name.is_empty() {
            toast.show(Toast::error("Name is required."));
            return;
        }
        let (Some(starts_at), Some(ends_at)) =
            (parse_input(&form_starts()), parse_input(&form_ends()))
        else {
            toast.show(Toast::error("Start and end must both be set."));
            return;
        };
        if ends_at <= starts_at {
            toast.show(Toast::error("End must be after start."));
            return;
        }
        let is_current = form_current();
        let edit_id = modal.get_target();

        modal.start_submit();
        spawn(async move {
            let client = ApiClient::web();
            let result = if let Some(id) = edit_id {
                let body = UpdateSeasonRequest {
                    name: Some(name),
                    starts_at: Some(starts_at),
                    ends_at: Some(ends_at),
                    is_current: Some(is_current),
                };
                client
                    .put_json::<_, Season>(&format!("/api/admin/seasons/{id}"), &body)
                    .await
            } else {
                let body = CreateSeasonRequest {
                    name,
                    starts_at,
                    ends_at,
                    is_current,
                };
                client
                    .post_json::<_, Season>("/api/admin/seasons", &body)
                    .await
            };

            modal.end_submit();
            match result {
                Ok(_) => {
                    toast.show(Toast::success("Season saved."));
                    modal.close();
                    seasons.refresh += 1;
                }
                Err(e) => {
                    toast.show(Toast::error(format!("Failed to save season: {e}")));
                }
            }
        });
    };

    let on_confirm_delete = move |_| {
        let Some(target) = delete_modal.get_target() else {
            return;
        };
        delete_modal.close();
        spawn(async move {
            match ApiClient::web()
                .delete(&format!("/api/admin/seasons/{}", target.id))
                .await
            {
                Ok(_) => {
                    toast.show(Toast::success("Season deleted."));
                    seasons.refresh += 1;
                }
                Err(e) => {
                    toast.show(Toast::error(format!("Failed to delete season: {e}")));
                }
            }
        });
    };

    let on_cancel_delete = move |_| {
        delete_modal.close();
    };

    if !auth().is_admin() {
        return rsx! {
            AccessDenied { message: "You need admin permissions to manage seasons.".to_string() }
        };
    }

    rsx! {
        div { class: "admin-toolbar",
            h1 { "Seasons" }
            button { class: "btn-add", onclick: open_create, "+ Add Season" }
        }
        p { class: "empty-state",
            "Seasons split every stats page into \"All time\" or one window. "
            "A game belongs to the season whose start ≤ played time < end (UTC). "
            "Editing or deleting a season regroups existing games; nothing is lost."
        }

        {
            let data = seasons.data.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => admin_pending(&seasons, "seasons"),
                Some(list) if list.is_empty() => rsx! {
                    p { class: "empty-state", "No seasons yet — stats show all time until one exists." }
                },
                Some(list) => rsx! {
                    DataTable { headers: vec!["Name", "Starts (UTC)", "Ends (UTC)", "Current", "Actions"],
                        for season in list.iter() {
                            {
                                let e = season.clone();
                                let d = season.clone();
                                let starts = fmt_display(&season.starts_at);
                                let ends = fmt_display(&season.ends_at);
                                rsx! {
                                    tr { key: "{season.id}",
                                        td { "{season.name}" }
                                        td { "{starts}" }
                                        td { "{ends}" }
                                        td { if season.is_current { "Yes" } else { "—" } }
                                        td {
                                            div { class: "row-actions",
                                                button {
                                                    class: "row-btn",
                                                    onclick: move |_| open_edit(e.clone()),
                                                    "Edit"
                                                }
                                                button {
                                                    class: "row-btn danger",
                                                    onclick: move |_| open_delete(d.clone()),
                                                    "Delete"
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
            title: if modal.get_target().is_some() { "Edit Season".to_string() } else { "Add Season".to_string() },
            open: modal.is_open(),
            submitting: modal.is_submitting(),
            on_close: on_close,
            on_submit: on_submit,

            div { class: "form-field",
                label { class: "form-label", "Name" }
                input {
                    class: "form-input",
                    r#type: "text",
                    placeholder: "e.g. Season 3",
                    value: "{form_name}",
                    oninput: move |e| form_name.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Starts (UTC)" }
                input {
                    class: "form-input",
                    r#type: "datetime-local",
                    value: "{form_starts}",
                    oninput: move |e| form_starts.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Ends (UTC, exclusive)" }
                input {
                    class: "form-input",
                    r#type: "datetime-local",
                    value: "{form_ends}",
                    oninput: move |e| form_ends.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label",
                    input {
                        r#type: "checkbox",
                        checked: form_current(),
                        onchange: move |e| form_current.set(e.checked()),
                    }
                    " Current season (clears the flag on any other season)"
                }
            }
        }

        ConfirmDialog {
            title: "Delete Season".to_string(),
            message: format!(
                "Delete \"{}\"? Games are kept; they simply stop being grouped under this season.",
                delete_modal.get_target().map(|s| s.name).unwrap_or_default()
            ),
            open: delete_modal.is_open(),
            danger: true,
            on_confirm: on_confirm_delete,
            on_cancel: on_cancel_delete,
        }
    }
}
