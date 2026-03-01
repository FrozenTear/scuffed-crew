use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};

use scuffed_ui::components::button::{Button, ButtonVariant};
use scuffed_ui::components::toast::{use_toast, Toast};

use crate::api;
use crate::components::confirm_dialog::ConfirmDialog;
use crate::components::data_table::DataTable;
use crate::components::forms::TextAreaField;
use crate::components::status_pill::StatusPill;

#[derive(Debug, Clone, Deserialize)]
struct Application {
    id: String,
    user_id: String,
    status: String,
    preferred_games: Vec<String>,
    message: Option<String>,
    #[allow(dead_code)]
    review_notes: Option<String>,
    #[allow(dead_code)]
    created_at: String,
}

#[derive(Serialize)]
struct UpdateApplicationBody {
    status: String,
    review_notes: Option<String>,
}

#[component]
pub fn ApplicationsPage() -> impl IntoView {
    let refresh = RwSignal::new(0u32);
    let toast = use_toast();

    let apps = LocalResource::new(move || {
        refresh.get();
        async { api::get::<Vec<Application>>("/api/applications").await.ok() }
    });

    // Reject dialog state
    let reject_open = RwSignal::new(false);
    let reject_id = RwSignal::new(String::new());
    let reject_notes = RwSignal::new(String::new());

    let do_accept = move |id: String| {
        spawn_local(async move {
            let body = UpdateApplicationBody {
                status: "accepted".into(),
                review_notes: None,
            };
            match api::patch::<_, Application>(&format!("/api/applications/{id}"), &body).await {
                Ok(_) => {
                    toast.show(Toast::success("Application accepted"));
                    refresh.update(|n| *n += 1);
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
        });
    };

    let do_reject = move || {
        let id = reject_id.get();
        let notes = reject_notes.get();
        spawn_local(async move {
            let body = UpdateApplicationBody {
                status: "rejected".into(),
                review_notes: if notes.is_empty() { None } else { Some(notes) },
            };
            match api::patch::<_, Application>(&format!("/api/applications/{id}"), &body).await {
                Ok(_) => {
                    toast.show(Toast::success("Application rejected"));
                    reject_open.set(false);
                    reject_notes.set(String::new());
                    refresh.update(|n| *n += 1);
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
        });
    };

    view! {
        <h1>"Applications"</h1>
        {move || match apps.get().flatten() {
            None => view! { <p>"Loading..."</p> }.into_any(),
            Some(list) if list.is_empty() => view! { <p>"No applications."</p> }.into_any(),
            Some(list) => view! {
                <DataTable headers=vec!["User", "Games", "Status", "Message", "Actions"]>
                    {list.into_iter().map(|a| {
                        let games = a.preferred_games.join(", ");
                        let msg = a.message.unwrap_or_else(|| "\u{2014}".into());
                        let is_pending = a.status == "pending";
                        let status = a.status.clone();
                        let id = a.id.clone();
                        let id2 = a.id.clone();
                        view! {
                            <tr>
                                <td>{a.user_id}</td>
                                <td>{games}</td>
                                <td><StatusPill status=status/></td>
                                <td>{msg}</td>
                                <td class="table-actions">
                                    {if is_pending {
                                        let accept_id = id.clone();
                                        Some(view! {
                                            <Button
                                                variant=ButtonVariant::Primary
                                                on_click=Callback::new(move |_| {
                                                    do_accept(accept_id.clone());
                                                })
                                            >
                                                "Accept"
                                            </Button>
                                            <Button
                                                variant=ButtonVariant::Danger
                                                on_click=Callback::new(move |_| {
                                                    reject_id.set(id2.clone());
                                                    reject_notes.set(String::new());
                                                    reject_open.set(true);
                                                })
                                            >
                                                "Reject"
                                            </Button>
                                        })
                                    } else {
                                        None
                                    }}
                                </td>
                            </tr>
                        }
                    }).collect_view()}
                </DataTable>
            }.into_any(),
        }}

        <ConfirmDialog
            open=reject_open
            on_confirm=Callback::new(move |_| do_reject())
            on_cancel=Callback::new(move |_| reject_open.set(false))
            title="Reject Application".to_string()
            message="Are you sure you want to reject this application?".to_string()
            danger=true
        >
            <div class="admin-form" style="margin-top: 0.75rem;">
                <TextAreaField label="Review Notes (optional)" value=reject_notes rows=3/>
            </div>
        </ConfirmDialog>
    }
}
