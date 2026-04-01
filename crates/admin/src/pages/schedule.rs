use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};

use scuffed_ui::components::button::{Button, ButtonVariant};
use scuffed_ui::components::modal::Modal;
use scuffed_ui::components::toast::{use_toast, Toast};

use crate::api;
use crate::components::confirm_dialog::ConfirmDialog;
use crate::components::data_table::DataTable;
use crate::components::form_modal::FormModal;
use crate::components::forms::{event_target_checked, CheckboxField, FormField, SelectField};

#[derive(Debug, Clone, Deserialize)]
struct Event {
    id: String,
    title: String,
    day_of_week: u8,
    time: String,
    timezone: String,
    is_recurring: bool,
    team_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Team {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RsvpSummary {
    #[allow(dead_code)]
    event_id: String,
    yes_count: u32,
    maybe_count: u32,
    no_count: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct Member {
    id: String,
    display_name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct EventAttendance {
    #[allow(dead_code)]
    id: String,
    member_id: String,
    #[allow(dead_code)]
    event_id: String,
    status: String,
}

#[derive(Serialize)]
struct AttendanceEntry {
    member_id: String,
    status: String,
}

#[derive(Serialize)]
struct BatchAttendanceBody {
    occurrence_date: String,
    entries: Vec<AttendanceEntry>,
}

#[derive(Serialize)]
struct CreateEventBody {
    title: String,
    day_of_week: u8,
    time: String,
    timezone: String,
    is_recurring: bool,
    team_id: Option<String>,
}

#[derive(Serialize)]
struct UpdateEventBody {
    title: Option<String>,
    day_of_week: Option<u8>,
    time: Option<String>,
    timezone: Option<String>,
    is_recurring: Option<bool>,
    team_id: Option<Option<String>>,
}

fn day_name(d: u8) -> &'static str {
    match d {
        0 => "Monday",
        1 => "Tuesday",
        2 => "Wednesday",
        3 => "Thursday",
        4 => "Friday",
        5 => "Saturday",
        6 => "Sunday",
        _ => "?",
    }
}

#[component]
pub fn SchedulePage() -> impl IntoView {
    let refresh = RwSignal::new(0u32);
    let toast = use_toast();

    let events = LocalResource::new(move || {
        refresh.get();
        async { api::get_list::<Event>("/api/events").await.ok() }
    });

    let teams_list =
        LocalResource::new(|| async { api::get_list::<Team>("/api/teams").await.ok() });

    // Form modal state
    let form_open = RwSignal::new(false);
    let form_editing_id = RwSignal::new(Option::<String>::None);
    let form_title = RwSignal::new(String::new());
    let form_day = RwSignal::new("0".to_string());
    let form_time = RwSignal::new(String::new());
    let form_timezone = RwSignal::new("CET".to_string());
    let form_recurring = RwSignal::new(true);
    let form_team_id = RwSignal::new(String::new());
    let form_submitting = RwSignal::new(false);

    // Delete confirm state
    let delete_open = RwSignal::new(false);
    let delete_id = RwSignal::new(String::new());

    let open_create = move || {
        form_editing_id.set(None);
        form_title.set(String::new());
        form_day.set("0".to_string());
        form_time.set(String::new());
        form_timezone.set("CET".to_string());
        form_recurring.set(true);
        form_team_id.set(String::new());
        form_open.set(true);
    };

    let open_edit = move |e: &Event| {
        form_editing_id.set(Some(e.id.clone()));
        form_title.set(e.title.clone());
        form_day.set(e.day_of_week.to_string());
        form_time.set(e.time.clone());
        form_timezone.set(e.timezone.clone());
        form_recurring.set(e.is_recurring);
        form_team_id.set(e.team_id.clone().unwrap_or_default());
        form_open.set(true);
    };

    let do_submit = move || {
        let editing_id = form_editing_id.get();
        let title = form_title.get();
        let day: u8 = form_day.get().parse().unwrap_or(0);
        let time = form_time.get();
        let timezone = form_timezone.get();
        let recurring = form_recurring.get();
        let team_id = form_team_id.get();
        let team_id_opt = if team_id.is_empty() {
            None
        } else {
            Some(team_id)
        };
        form_submitting.set(true);

        spawn_local(async move {
            let result = if let Some(id) = editing_id {
                let body = UpdateEventBody {
                    title: Some(title),
                    day_of_week: Some(day),
                    time: Some(time),
                    timezone: Some(timezone),
                    is_recurring: Some(recurring),
                    team_id: Some(team_id_opt),
                };
                api::put::<_, Event>(&format!("/api/events/{id}"), &body)
                    .await
                    .map(|_| "Event updated")
            } else {
                let body = CreateEventBody {
                    title,
                    day_of_week: day,
                    time,
                    timezone,
                    is_recurring: recurring,
                    team_id: team_id_opt,
                };
                api::post::<_, Event>("/api/events", &body)
                    .await
                    .map(|_| "Event created")
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
            match api::delete(&format!("/api/events/{id}")).await {
                Ok(_) => {
                    toast.show(Toast::success("Event deleted"));
                    delete_open.set(false);
                    refresh.update(|n| *n += 1);
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
        });
    };

    // Attendance modal state
    let att_open = RwSignal::new(false);
    let att_event_id = RwSignal::new(String::new());
    let att_event_title = RwSignal::new(String::new());
    let att_date = RwSignal::new(String::new());
    let att_members = RwSignal::new(Vec::<Member>::new());
    let att_statuses = RwSignal::new(Vec::<(String, RwSignal<String>)>::new());
    let att_submitting = RwSignal::new(false);

    let open_attendance = move |event_id: String, event_title: String| {
        att_event_id.set(event_id);
        att_event_title.set(event_title);
        // Default to today
        let today = js_sys::Date::new_0();
        let y = today.get_full_year();
        let m = today.get_month() + 1;
        let d = today.get_date();
        att_date.set(format!("{y:04}-{m:02}-{d:02}"));
        att_open.set(true);
        // Fetch member list
        spawn_local(async move {
            match api::get_list::<Member>("/api/members").await {
                Ok(members) => {
                    let statuses: Vec<(String, RwSignal<String>)> = members
                        .iter()
                        .map(|m| (m.id.clone(), RwSignal::new("attended".to_string())))
                        .collect();
                    att_statuses.set(statuses);
                    att_members.set(members);
                }
                Err(e) => toast.show(Toast::error(format!("Failed to load members: {e}"))),
            }
        });
    };

    let do_submit_attendance = move || {
        let event_id = att_event_id.get();
        let date = att_date.get();
        let statuses = att_statuses.get();

        if date.is_empty() {
            toast.show(Toast::error("Date is required"));
            return;
        }

        let entries: Vec<AttendanceEntry> = statuses
            .iter()
            .map(|(mid, sig)| AttendanceEntry {
                member_id: mid.clone(),
                status: sig.get(),
            })
            .collect();

        let body = BatchAttendanceBody {
            occurrence_date: format!("{date}T00:00:00Z"),
            entries,
        };

        att_submitting.set(true);
        spawn_local(async move {
            match api::post::<_, Vec<serde_json::Value>>(
                &format!("/api/events/{event_id}/attendance"),
                &body,
            ).await {
                Ok(_) => {
                    toast.show(Toast::success("Attendance recorded"));
                    att_open.set(false);
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
            att_submitting.set(false);
        });
    };

    let modal_title = Signal::derive(move || {
        if form_editing_id.get().is_some() {
            "Edit Event".to_string()
        } else {
            "Create Event".to_string()
        }
    });

    let att_modal_title = Signal::derive(move || {
        format!("Attendance \u{2014} {}", att_event_title.get())
    });

    view! {
        <h1>"Schedule"</h1>
        <div class="page-actions">
            <Button
                variant=ButtonVariant::Primary
                on_click=Callback::new(move |_| open_create())
            >
                "Create Event"
            </Button>
        </div>
        {move || match events.get().flatten() {
            None => view! { <p>"Loading..."</p> }.into_any(),
            Some(list) if list.is_empty() => view! { <p>"No events scheduled."</p> }.into_any(),
            Some(list) => view! {
                <DataTable headers=vec!["Title", "Day", "Time", "RSVPs", "Recurring", "Actions"]>
                    {list.into_iter().map(|e| {
                        let recurring = if e.is_recurring { "Yes" } else { "No" };
                        let time_display = format!("{} {}", e.time, e.timezone);
                        let e_edit = e.clone();
                        let e_id = e.id.clone();
                        let e_id_att = e.id.clone();
                        let e_title_att = e.title.clone();
                        view! {
                            <tr>
                                <td>{e.title.clone()}</td>
                                <td>{day_name(e.day_of_week)}</td>
                                <td>{time_display}</td>
                                <td><RsvpCell event_id=e.id.clone()/></td>
                                <td>{recurring}</td>
                                <td class="table-actions">
                                    <Button
                                        variant=ButtonVariant::Secondary
                                        on_click=Callback::new(move |_| open_edit(&e_edit))
                                    >
                                        "Edit"
                                    </Button>
                                    <Button
                                        variant=ButtonVariant::Secondary
                                        on_click=Callback::new(move |_| {
                                            open_attendance(e_id_att.clone(), e_title_att.clone());
                                        })
                                    >
                                        "Attendance"
                                    </Button>
                                    <Button
                                        variant=ButtonVariant::Danger
                                        on_click=Callback::new(move |_| {
                                            delete_id.set(e_id.clone());
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

        // Attendance modal
        <Modal open=att_open on_close=Callback::new(move |_| att_open.set(false))>
            <div style="min-width: 500px; max-width: 700px;">
                <h3 style="font-family: var(--font-display); font-size: 1.2rem; color: var(--text-bright); text-transform: uppercase; margin: 0 0 1rem 0;">
                    {move || att_modal_title.get()}
                </h3>
                <FormField label="Occurrence Date" value=att_date input_type="date"/>
                <div style="margin-top: 1rem; max-height: 400px; overflow-y: auto;">
                    {move || {
                        let members = att_members.get();
                        let statuses = att_statuses.get();
                        if members.is_empty() {
                            view! { <p style="color: var(--text-muted);">"Loading members..."</p> }.into_any()
                        } else {
                            view! {
                                <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                                    {members.into_iter().zip(statuses.into_iter()).map(|(m, (_mid, sig))| {
                                        let is_attended = Signal::derive(move || sig.get() == "attended");
                                        let is_excused = Signal::derive(move || sig.get() == "excused");
                                        view! {
                                            <div style="display: flex; align-items: center; gap: 0.75rem; padding: 0.5rem 0.75rem; background: var(--bg-surface); border-radius: 6px; border: 1px solid var(--border);">
                                                <label style="display: flex; align-items: center; gap: 0.5rem; flex: 1; cursor: pointer; color: var(--text-primary); font-size: 0.9rem;">
                                                    <input
                                                        type="checkbox"
                                                        style="width: 1.1rem; height: 1.1rem; accent-color: #4ade80; cursor: pointer;"
                                                        prop:checked=move || is_attended.get()
                                                        prop:disabled=move || is_excused.get()
                                                        on:change=move |ev| {
                                                            let checked: bool = event_target_checked(&ev);
                                                            if checked {
                                                                sig.set("attended".to_string());
                                                            } else {
                                                                sig.set("no_show".to_string());
                                                            }
                                                        }
                                                    />
                                                    <span style:opacity=move || if is_excused.get() { "0.5" } else { "1" }>
                                                        {m.display_name}
                                                    </span>
                                                </label>
                                                <button
                                                    style=move || format!(
                                                        "background: {}; color: {}; border: 1px solid {}; border-radius: 4px; padding: 0.2rem 0.5rem; font-size: 0.75rem; cursor: pointer; white-space: nowrap;",
                                                        if is_excused.get() { "rgba(251, 191, 36, 0.15)" } else { "transparent" },
                                                        if is_excused.get() { "#fbbf24" } else { "var(--text-muted)" },
                                                        if is_excused.get() { "#fbbf24" } else { "var(--border)" },
                                                    )
                                                    on:click=move |_| {
                                                        if is_excused.get() {
                                                            sig.set("attended".to_string());
                                                        } else {
                                                            sig.set("excused".to_string());
                                                        }
                                                    }
                                                >
                                                    "Excused"
                                                </button>
                                            </div>
                                        }
                                    }).collect_view()}
                                </div>
                            }.into_any()
                        }
                    }}
                </div>
                <div style="display: flex; justify-content: flex-end; gap: 0.75rem; margin-top: 1rem;">
                    <Button
                        variant=ButtonVariant::Secondary
                        on_click=Callback::new(move |_| att_open.set(false))
                    >
                        "Cancel"
                    </Button>
                    <Button
                        variant=ButtonVariant::Primary
                        disabled=att_submitting.get()
                        on_click=Callback::new(move |_| do_submit_attendance())
                    >
                        {move || if att_submitting.get() { "Saving..." } else { "Save Attendance" }}
                    </Button>
                </div>
            </div>
        </Modal>

        <FormModal
            open=form_open
            on_close=Callback::new(move |_| form_open.set(false))
            title=modal_title
            on_submit=Callback::new(move |_| do_submit())
            submitting=form_submitting
        >
            <FormField label="Title" value=form_title/>
            <SelectField
                label="Day"
                value=form_day
                options=vec![
                    ("0", "Monday"), ("1", "Tuesday"), ("2", "Wednesday"),
                    ("3", "Thursday"), ("4", "Friday"), ("5", "Saturday"), ("6", "Sunday"),
                ]
            />
            <FormField label="Time (e.g. 20:00)" value=form_time/>
            <SelectField
                label="Timezone"
                value=form_timezone
                options=vec![
                    ("CET", "CET"), ("CEST", "CEST"), ("GMT", "GMT"), ("UTC", "UTC"),
                    ("EST", "EST"), ("PST", "PST"),
                ]
            />
            <CheckboxField label="Recurring" value=form_recurring/>
            // Team selector (optional)
            <div>
                <label style="color: var(--text-secondary); font-size: 0.85rem;">"Team (optional)"</label>
                <select
                    class="admin-form-select"
                    style="background: var(--bg-surface); border: 1px solid var(--border); border-radius: 6px; padding: 0.5rem 0.75rem; color: var(--text-primary); width: 100%; font-size: 0.9rem;"
                    prop:value=move || form_team_id.get()
                    on:change=move |ev| form_team_id.set(event_target_value(&ev))
                >
                    <option value="">"None"</option>
                    {move || teams_list.get().flatten().unwrap_or_default().into_iter().map(|t| {
                        view! { <option value={t.id.clone()}>{t.name}</option> }
                    }).collect_view()}
                </select>
            </div>
        </FormModal>

        <ConfirmDialog
            open=delete_open
            on_confirm=Callback::new(move |_| do_delete())
            on_cancel=Callback::new(move |_| delete_open.set(false))
            title="Delete Event".to_string()
            message="Are you sure you want to delete this event?".to_string()
            danger=true
        />
    }
}

/// Inline RSVP summary cell for the schedule table.
#[component]
fn RsvpCell(event_id: String) -> impl IntoView {
    let summary = LocalResource::new(move || {
        let eid = event_id.clone();
        async move {
            api::get::<RsvpSummary>(&format!("/api/events/{eid}/rsvp-summary"))
                .await
                .ok()
        }
    });

    view! {
        {move || match summary.get().flatten() {
            None => view! { <span style="color: var(--text-muted);">"\u{2014}"</span> }.into_any(),
            Some(s) => view! {
                <span style="font-size: 0.85rem; white-space: nowrap;">
                    <span style="color: #4ade80;">{s.yes_count}</span>
                    " / "
                    <span style="color: #fbbf24;">{s.maybe_count}</span>
                    " / "
                    <span style="color: #9ca3af;">{s.no_count}</span>
                </span>
            }.into_any(),
        }}
    }
}
