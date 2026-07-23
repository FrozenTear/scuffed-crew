use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::modal::Modal;
use crate::components::{ConfirmDialog, DataTable, StatusPill, Toast, admin_pending, use_toast};
use crate::hooks::{ModalController, use_api_list};
use crate::util::format_datetime;
use scuffed_api_client::ApiClient;
use scuffed_types::api::PatchApplicationRequest;

// Matches the enriched ApplicationListEntry JSON from GET /api/applications.
// Name/label fields are optional so the page still renders against an older server.
#[derive(Debug, Clone, Deserialize)]
struct Application {
    id: String,
    user_id: String,
    #[serde(default)]
    applicant_name: Option<String>,
    preferred_games: Vec<String>,
    #[serde(default)]
    preferred_game_names: Option<Vec<String>>,
    #[serde(default)]
    preferred_roles: Vec<String>,
    message: Option<String>,
    status: String,
    #[serde(default)]
    review_notes: Option<String>,
    created_at: String,
    #[serde(default)]
    updated_at: Option<String>,
}

impl Application {
    fn applicant_label(&self) -> String {
        self.applicant_name
            .clone()
            .unwrap_or_else(|| self.user_id.clone())
    }

    fn games_label(&self) -> String {
        match &self.preferred_game_names {
            Some(names) if !names.is_empty() => names.join(", "),
            _ => self.preferred_games.join(", "),
        }
    }
}

#[component]
pub fn AdminApplications() -> Element {
    // Cursor-paginated list (auto-follows pages via use_api_list).
    let mut applications = use_api_list::<Application>("/api/applications");
    let mut toast = use_toast();

    // Reject dialog state
    let mut reject_modal = ModalController::<String>::new();
    let mut reject_notes = use_signal(String::new);

    // Read-only detail drawer (any application, incl. closed ones)
    let mut view_open = use_signal(|| false);
    let mut view_target = use_signal(|| None::<Application>);

    let accept = move |id: String| {
        spawn(async move {
            let body = PatchApplicationRequest {
                status: "accepted".to_string(),
                review_notes: None,
            };
            let path = format!("/api/applications/{id}");
            match ApiClient::web()
                .patch_json::<_, Application>(&path, &body)
                .await
            {
                Ok(_) => {
                    toast.show(Toast::success("Application accepted"));
                    applications.refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Failed to accept: {e}"))),
            }
        });
    };

    let confirm_reject = move |_| {
        let id = reject_modal.get_target().unwrap_or_default();
        let notes = reject_notes().clone();
        reject_modal.close();
        reject_notes.set(String::new());
        spawn(async move {
            let body = PatchApplicationRequest {
                status: "rejected".to_string(),
                review_notes: if notes.is_empty() { None } else { Some(notes) },
            };
            let path = format!("/api/applications/{id}");
            match ApiClient::web()
                .patch_json::<_, Application>(&path, &body)
                .await
            {
                Ok(_) => {
                    toast.show(Toast::success("Application rejected"));
                    applications.refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Failed to reject: {e}"))),
            }
        });
    };

    rsx! {

        h1 { "Applications" }

        {
            let data = applications.data.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => admin_pending(&applications, "applications"),
                Some(list) if list.is_empty() => rsx! {
                    p { class: "empty-state", "No applications." }
                },
                Some(list) => rsx! {
                    DataTable { headers: vec!["Applicant", "Games", "Message", "Status", "Date", "Actions"],
                        for app in list.iter() {
                            {
                                let id = app.id.clone();
                                let id2 = app.id.clone();
                                let applicant = app.applicant_label();
                                let games = app.games_label();
                                let msg = app.message.clone().unwrap_or_default();
                                let date: String = app.created_at.chars().take(10).collect();
                                let is_pending = app.status == "pending";
                                let view_app = app.clone();
                                rsx! {
                                    tr { key: "{id}",
                                        td { "{applicant}" }
                                        td { "{games}" }
                                        td { "{msg}" }
                                        td { StatusPill { status: app.status.clone() } }
                                        td { "{date}" }
                                        td {
                                            div { class: "row-actions",
                                                if is_pending {
                                                    button {
                                                        class: "row-btn primary",
                                                        onclick: move |_| accept(id.clone()),
                                                        "Accept"
                                                    }
                                                    button {
                                                        class: "row-btn danger",
                                                        onclick: move |_| reject_modal.show(id2.clone()),
                                                        "Reject"
                                                    }
                                                }
                                                button {
                                                    class: "row-btn",
                                                    onclick: move |_| {
                                                        view_target.set(Some(view_app.clone()));
                                                        view_open.set(true);
                                                    },
                                                    "View"
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

        Modal {
            open: view_open,
            on_close: move |_| view_open.set(false),
            {
                match view_target() {
                    Some(app) => {
                        let roles = if app.preferred_roles.is_empty() {
                            "—".to_string()
                        } else {
                            app.preferred_roles.join(", ")
                        };
                        let message = app.message.clone().unwrap_or_else(|| "—".into());
                        let notes = app.review_notes.clone().unwrap_or_else(|| "—".into());
                        let submitted = format_datetime(&app.created_at);
                        let updated = app
                            .updated_at
                            .as_deref()
                            .map(format_datetime)
                            .unwrap_or_else(|| "—".into());
                        rsx! {
                            div { class: "application-detail",
                                h2 { "Application: {app.applicant_label()}" }
                                dl {
                                    dt { "Status" }
                                    dd { StatusPill { status: app.status.clone() } }
                                    dt { "Games" }
                                    dd { "{app.games_label()}" }
                                    dt { "Roles" }
                                    dd { "{roles}" }
                                    dt { "Message" }
                                    dd { "{message}" }
                                    dt { "Review notes" }
                                    dd { "{notes}" }
                                    dt { "Submitted" }
                                    dd { "{submitted}" }
                                    dt { "Last update" }
                                    dd { "{updated}" }
                                }
                            }
                        }
                    }
                    None => rsx! {},
                }
            }
        }

        ConfirmDialog {
            title: "Reject Application".to_string(),
            message: "Are you sure you want to reject this application?".to_string(),
            open: reject_modal.is_open(),
            danger: true,
            on_confirm: confirm_reject,
            on_cancel: move |_| {
                reject_modal.close();
                reject_notes.set(String::new());
            },
            extra: rsx! {
                div { class: "form-field", style: "margin-top: 0.75rem;",
                    label { class: "form-label", "Rejection Notes (optional)" }
                    textarea {
                        class: "form-textarea",
                        value: "{reject_notes}",
                        oninput: move |e| reject_notes.set(e.value()),
                    }
                }
            },
        }
    }
}
