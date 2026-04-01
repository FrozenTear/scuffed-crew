use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};

use scuffed_ui::components::button::{Button, ButtonVariant};
use scuffed_ui::components::toast::{use_toast, Toast};

use crate::api;
use crate::components::confirm_dialog::ConfirmDialog;
use crate::components::data_table::DataTable;
use crate::components::form_modal::FormModal;
use crate::components::forms::{CheckboxField, FormField, TextAreaField};

#[derive(Debug, Clone, Deserialize)]
struct Announcement {
    id: String,
    title: String,
    content: String,
    author_id: String,
    pinned: bool,
    #[allow(dead_code)]
    is_active: bool,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
}

#[derive(Serialize)]
struct CreateAnnouncementBody {
    title: String,
    content: String,
    pinned: bool,
}

#[derive(Serialize)]
struct UpdateAnnouncementBody {
    title: Option<String>,
    content: Option<String>,
    pinned: Option<bool>,
}

#[component]
pub fn AnnouncementsPage() -> impl IntoView {
    let refresh = RwSignal::new(0u32);
    let toast = use_toast();

    let announcements = LocalResource::new(move || {
        refresh.get();
        async { api::get_list::<Announcement>("/api/announcements").await.ok() }
    });

    // Form modal state
    let form_open = RwSignal::new(false);
    let form_editing_id = RwSignal::new(Option::<String>::None);
    let form_title = RwSignal::new(String::new());
    let form_content = RwSignal::new(String::new());
    let form_pinned = RwSignal::new(false);
    let form_submitting = RwSignal::new(false);

    // Delete confirm state
    let delete_open = RwSignal::new(false);
    let delete_id = RwSignal::new(String::new());

    let open_create = move || {
        form_editing_id.set(None);
        form_title.set(String::new());
        form_content.set(String::new());
        form_pinned.set(false);
        form_open.set(true);
    };

    let open_edit = move |a: &Announcement| {
        form_editing_id.set(Some(a.id.clone()));
        form_title.set(a.title.clone());
        form_content.set(a.content.clone());
        form_pinned.set(a.pinned);
        form_open.set(true);
    };

    let do_submit = move || {
        let editing_id = form_editing_id.get();
        let title = form_title.get();
        let content = form_content.get();
        let pinned = form_pinned.get();
        form_submitting.set(true);

        spawn_local(async move {
            let result = if let Some(id) = editing_id {
                let body = UpdateAnnouncementBody {
                    title: Some(title),
                    content: Some(content),
                    pinned: Some(pinned),
                };
                api::put::<_, Announcement>(&format!("/api/announcements/{id}"), &body)
                    .await
                    .map(|_| "Announcement updated")
            } else {
                let body = CreateAnnouncementBody {
                    title,
                    content,
                    pinned,
                };
                api::post::<_, Announcement>("/api/announcements", &body)
                    .await
                    .map(|_| "Announcement created")
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

    let do_delete = move || {
        let id = delete_id.get();
        spawn_local(async move {
            match api::delete(&format!("/api/announcements/{id}")).await {
                Ok(_) => {
                    toast.show(Toast::success("Announcement deleted"));
                    delete_open.set(false);
                    refresh.update(|n| *n += 1);
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
        });
    };

    let modal_title = Signal::derive(move || {
        if form_editing_id.get().is_some() {
            "Edit Announcement".to_string()
        } else {
            "New Announcement".to_string()
        }
    });

    view! {
        <h1>"Announcements"</h1>
        <div class="page-actions">
            <Button
                variant=ButtonVariant::Primary
                on_click=Callback::new(move |_| open_create())
            >
                "New Announcement"
            </Button>
        </div>
        {move || match announcements.get().flatten() {
            None => view! { <p>"Loading..."</p> }.into_any(),
            Some(list) if list.is_empty() => view! { <p>"No announcements yet."</p> }.into_any(),
            Some(list) => view! {
                <DataTable headers=vec!["Title", "Author", "Pinned", "Date", "Actions"]>
                    {list.into_iter().map(|a| {
                        let pinned_str = if a.pinned { "Yes" } else { "No" };
                        let date = a.created_at.chars().take(10).collect::<String>();
                        let a_edit = a.clone();
                        let a_id = a.id.clone();
                        view! {
                            <tr>
                                <td>{a.title.clone()}</td>
                                <td>{a.author_id.clone()}</td>
                                <td>{pinned_str}</td>
                                <td>{date}</td>
                                <td class="table-actions">
                                    <Button
                                        variant=ButtonVariant::Secondary
                                        on_click=Callback::new(move |_| open_edit(&a_edit))
                                    >
                                        "Edit"
                                    </Button>
                                    <Button
                                        variant=ButtonVariant::Danger
                                        on_click=Callback::new(move |_| {
                                            delete_id.set(a_id.clone());
                                            delete_open.set(true);
                                        })
                                    >
                                        "Delete"
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
            title=modal_title
            on_submit=Callback::new(move |_| do_submit())
            submitting=form_submitting
        >
            <FormField label="Title" value=form_title/>
            <TextAreaField label="Content" value=form_content rows=6/>
            <CheckboxField label="Pinned" value=form_pinned/>
        </FormModal>

        <ConfirmDialog
            open=delete_open
            on_confirm=Callback::new(move |_| do_delete())
            on_cancel=Callback::new(move |_| delete_open.set(false))
            title="Delete Announcement".to_string()
            message="Are you sure you want to delete this announcement?".to_string()
            danger=true
        />
    }
}
