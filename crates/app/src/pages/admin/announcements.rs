use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::{ConfirmDialog, DataTable, FormModal, Toast, use_toast};
use crate::hooks::{ModalController, use_api_list};
use scuffed_api_client::ApiClient;
use scuffed_types::api::{CreateAnnouncementRequest, UpdateAnnouncementRequest};

// Local response type (field name `is_pinned` differs from shared `pinned`).
#[derive(Debug, Clone, Deserialize)]
struct Announcement {
    id: String,
    title: String,
    content: String,
    is_pinned: bool,
    created_at: String,
}

#[component]
pub fn AdminAnnouncements() -> Element {
    let mut announcements = use_api_list::<Announcement>("/api/announcements");
    let mut toast = use_toast();

    // Form modal state
    let mut modal = ModalController::<String>::new();
    let mut form_title = use_signal(String::new);
    let mut form_content = use_signal(String::new);
    let mut form_pinned = use_signal(|| false);

    // Delete confirm state
    let mut delete_modal = ModalController::<Announcement>::new();

    let open_create = move |_| {
        form_title.set(String::new());
        form_content.set(String::new());
        form_pinned.set(false);
        modal.show_empty();
    };

    let mut open_edit = move |a: Announcement| {
        form_title.set(a.title);
        form_content.set(a.content);
        form_pinned.set(a.is_pinned);
        modal.show(a.id);
    };

    let mut open_delete = move |a: Announcement| {
        delete_modal.show(a);
    };

    let on_close = move |_| {
        modal.close();
    };

    let on_submit = move |_| {
        let title = form_title().trim().to_string();
        let content = form_content().trim().to_string();
        if title.is_empty() || content.is_empty() {
            return;
        }
        let is_pinned = form_pinned();
        let edit_id = modal.get_target();

        modal.start_submit();
        spawn(async move {
            let client = ApiClient::web();
            let result = if let Some(id) = edit_id {
                let body = UpdateAnnouncementRequest {
                    title,
                    content,
                    is_pinned,
                };
                client
                    .put_json::<_, Announcement>(&format!("/api/announcements/{id}"), &body)
                    .await
            } else {
                let body = CreateAnnouncementRequest {
                    title,
                    content,
                    is_pinned,
                };
                client
                    .post_json::<_, Announcement>("/api/announcements", &body)
                    .await
            };

            modal.end_submit();
            match result {
                Ok(_) => {
                    toast.show(Toast::success("Announcement saved."));
                    modal.close();
                    announcements.refresh += 1;
                }
                Err(e) => {
                    toast.show(Toast::error(format!("Failed to save: {e}")));
                }
            }
        });
    };

    let on_confirm_delete = move |_| {
        let Some(target) = delete_modal.get_target() else {
            return;
        };
        let id = target.id.clone();
        delete_modal.close();
        spawn(async move {
            let client = ApiClient::web();
            match client.delete(&format!("/api/announcements/{id}")).await {
                Ok(_) => {
                    toast.show(Toast::success("Announcement deleted."));
                    announcements.refresh += 1;
                }
                Err(e) => {
                    toast.show(Toast::error(format!("Failed to delete: {e}")));
                }
            }
        });
    };

    let on_cancel_delete = move |_| {
        delete_modal.close();
    };

    rsx! {

        div { class: "admin-toolbar",
            h1 { "Announcements" }
            button { class: "btn-add", onclick: open_create, "+ New Announcement" }
        }

        {
            let data = announcements.data.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => rsx! { p { class: "admin-loading", "Loading..." } },
                Some(list) if list.is_empty() => rsx! {
                    p { class: "empty-state", "No announcements yet." }
                },
                Some(list) => rsx! {
                    DataTable { headers: vec!["Title", "Pinned", "Created", "Actions"],
                        for a in list.iter() {
                            {
                                let ae = a.clone();
                                let ad = a.clone();
                                let pinned_badge = if a.is_pinned { "active" } else { "inactive" };
                                let pinned_label = if a.is_pinned { "Yes" } else { "No" };
                                let date: String = a.created_at.chars().take(10).collect();
                                rsx! {
                                    tr { key: "{a.id}",
                                        td { "{a.title}" }
                                        td { span { class: "status-pill {pinned_badge}", "{pinned_label}" } }
                                        td { "{date}" }
                                        td {
                                            div { class: "row-actions",
                                                button {
                                                    class: "row-btn",
                                                    onclick: move |_| open_edit(ae.clone()),
                                                    "Edit"
                                                }
                                                button {
                                                    class: "row-btn danger",
                                                    onclick: move |_| open_delete(ad.clone()),
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
            title: if modal.get_target().is_some() { "Edit Announcement".to_string() } else { "New Announcement".to_string() },
            open: modal.is_open(),
            submitting: modal.is_submitting(),
            on_close: on_close,
            on_submit: on_submit,

            div { class: "form-field",
                label { class: "form-label", "Title" }
                input {
                    class: "form-input",
                    r#type: "text",
                    value: "{form_title}",
                    oninput: move |e| form_title.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Content" }
                textarea {
                    class: "form-textarea",
                    value: "{form_content}",
                    oninput: move |e| form_content.set(e.value()),
                }
            }
            div { class: "form-field",
                div { class: "form-checkbox-row",
                    input {
                        r#type: "checkbox",
                        checked: form_pinned(),
                        onchange: move |e| form_pinned.set(e.checked()),
                    }
                    label { class: "form-label", "Pinned" }
                }
            }
        }

        ConfirmDialog {
            title: "Delete Announcement".to_string(),
            message: format!("Are you sure you want to delete \"{}\"? This cannot be undone.", delete_modal.get_target().map(|a| a.title).unwrap_or_default()),
            open: delete_modal.is_open(),
            danger: true,
            on_confirm: on_confirm_delete,
            on_cancel: on_cancel_delete,
        }
    }
}
