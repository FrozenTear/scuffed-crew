use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use scuffed_api_client::ApiClient;
use crate::components::{DataTable, ConfirmDialog, StatusPill, Toast, use_toast, ADMIN_SHARED_CSS};

#[derive(Debug, Clone, Deserialize)]
struct Application {
    id: String,
    user_display_name: String,
    preferred_games: Vec<String>,
    message: Option<String>,
    status: String,
    created_at: String,
}

#[derive(Serialize)]
struct PatchApplication {
    status: String,
    review_notes: Option<String>,
}

#[component]
pub fn AdminApplications() -> Element {
    let mut refresh = use_signal(|| 0u64);
    let mut toast = use_toast();

    let applications = use_resource(move || async move {
        let _ = refresh();
        ApiClient::web().fetch::<Vec<Application>>("/api/applications").await.ok()
    });

    // Reject dialog state
    let mut reject_id = use_signal(|| None::<String>);
    let mut reject_notes = use_signal(String::new);

    let accept = move |id: String| {
        spawn(async move {
            let body = PatchApplication {
                status: "accepted".to_string(),
                review_notes: None,
            };
            let path = format!("/api/applications/{id}");
            match ApiClient::web().patch_json::<_, Application>(&path, &body).await {
                Ok(_) => {
                    toast.show(Toast::success("Application accepted"));
                    refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Failed to accept: {e}"))),
            }
        });
    };

    let confirm_reject = move |_| {
        let id = reject_id().clone().unwrap_or_default();
        let notes = reject_notes().clone();
        reject_id.set(None);
        reject_notes.set(String::new());
        spawn(async move {
            let body = PatchApplication {
                status: "rejected".to_string(),
                review_notes: if notes.is_empty() { None } else { Some(notes) },
            };
            let path = format!("/api/applications/{id}");
            match ApiClient::web().patch_json::<_, Application>(&path, &body).await {
                Ok(_) => {
                    toast.show(Toast::success("Application rejected"));
                    refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Failed to reject: {e}"))),
            }
        });
    };

    rsx! {
        style { {ADMIN_SHARED_CSS} }

        h1 { "Applications" }

        {
            let data = applications.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => rsx! { p { class: "admin-loading", "Loading..." } },
                Some(list) if list.is_empty() => rsx! {
                    p { class: "empty-state", "No applications." }
                },
                Some(list) => rsx! {
                    DataTable { headers: vec!["Applicant", "Games", "Message", "Status", "Date", "Actions"],
                        for app in list.iter() {
                            {
                                let id = app.id.clone();
                                let id2 = app.id.clone();
                                let games = app.preferred_games.join(", ");
                                let msg = app.message.clone().unwrap_or_default();
                                let date: String = app.created_at.chars().take(10).collect();
                                let is_pending = app.status == "pending";
                                rsx! {
                                    tr { key: "{id}",
                                        td { "{app.user_display_name}" }
                                        td { "{games}" }
                                        td { "{msg}" }
                                        td { StatusPill { status: app.status.clone() } }
                                        td { "{date}" }
                                        td {
                                            if is_pending {
                                                div { class: "row-actions",
                                                    button {
                                                        class: "row-btn primary",
                                                        onclick: move |_| accept(id.clone()),
                                                        "Accept"
                                                    }
                                                    button {
                                                        class: "row-btn danger",
                                                        onclick: move |_| reject_id.set(Some(id2.clone())),
                                                        "Reject"
                                                    }
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

        ConfirmDialog {
            title: "Reject Application".to_string(),
            message: "Are you sure you want to reject this application?".to_string(),
            open: reject_id().is_some(),
            danger: true,
            on_confirm: confirm_reject,
            on_cancel: move |_| {
                reject_id.set(None);
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
