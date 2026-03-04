use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;
use scuffed_types::api::{
    CreateEventRequest, BatchAttendanceRequest,
    AttendanceEntry,
};
use crate::components::{DataTable, FormModal, ConfirmDialog, Toast, use_toast, ADMIN_SHARED_CSS};
use crate::hooks::{use_api, ModalController};

// --- Types ---
// Local response types with API-enriched fields (joined names).

#[derive(Debug, Clone, Deserialize)]
struct Event {
    id: String,
    title: String,
    day_of_week: u8,
    time: String,
    timezone: String,
    is_recurring: bool,
    team_id: Option<String>,
    team_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Team {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Member {
    id: String,
    display_name: String,
}

const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const TIMEZONES: [&str; 5] = ["UTC", "Europe/London", "Europe/Berlin", "US/Eastern", "US/Pacific"];

#[component]
pub fn AdminSchedule() -> Element {
    let mut events = use_api::<Vec<Event>>("/api/events");
    let mut teams = use_api::<Vec<Team>>("/api/teams");
    let mut members = use_api::<Vec<Member>>("/api/members");
    let mut toast = use_toast();

    // Form modal state
    let mut modal = ModalController::<String>::new();
    let mut form_title = use_signal(String::new);
    let mut form_day = use_signal(|| 0u8);
    let mut form_time = use_signal(|| "19:00".to_string());
    let mut form_tz = use_signal(|| "UTC".to_string());
    let mut form_recurring = use_signal(|| true);
    let mut form_team_id: Signal<Option<String>> = use_signal(|| None);

    // Delete confirm state
    let mut delete_modal = ModalController::<Event>::new();

    // Attendance modal state
    let mut att_modal = ModalController::<Event>::new();
    let mut att_date = use_signal(String::new);
    let mut att_entries: Signal<Vec<AttendanceEntry>> = use_signal(Vec::new);


    // --- Event form handlers ---

    let open_create = move |_| {
        form_title.set(String::new());
        form_day.set(0);
        form_time.set("19:00".to_string());
        form_tz.set("UTC".to_string());
        form_recurring.set(true);
        form_team_id.set(None);
        modal.show_empty();
    };

    let mut open_edit = move |evt: Event| {
        form_title.set(evt.title);
        form_day.set(evt.day_of_week);
        form_time.set(evt.time);
        form_tz.set(evt.timezone);
        form_recurring.set(evt.is_recurring);
        form_team_id.set(evt.team_id);
        modal.show(evt.id);
    };

    let on_close = move |_| modal.close();

    let on_submit = move |_| {
        let title = form_title().trim().to_string();
        if title.is_empty() {
            return;
        }
        let body = CreateEventRequest {
            title,
            day_of_week: form_day(),
            time: form_time().trim().to_string(),
            timezone: form_tz(),
            is_recurring: form_recurring(),
            team_id: form_team_id(),
        };
        let edit_id = modal.get_target();
        modal.start_submit();
        spawn(async move {
            let client = ApiClient::web();
            let result = if let Some(id) = edit_id {
                client.put_json::<_, Event>(&format!("/api/events/{id}"), &body).await
            } else {
                client.post_json::<_, Event>("/api/events", &body).await
            };
            modal.end_submit();
            match result {
                Ok(_) => {
                    toast.show(Toast::success("Event saved."));
                    modal.close();
                    events.refresh += 1;
                    teams.refresh += 1;
                    members.refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Failed to save event: {e}"))),
            }
        });
    };

    // --- Delete handlers ---

    let mut open_delete = move |evt: Event| {
        delete_modal.show(evt);
    };

    let on_delete_confirm = move |_| {
        if let Some(evt) = delete_modal.get_target() {
            let id = evt.id.clone();
            delete_modal.close();
            spawn(async move {
                match ApiClient::web().delete(&format!("/api/events/{id}")).await {
                    Ok(_) => {
                        toast.show(Toast::success("Event deleted."));
                        events.refresh += 1;
                        teams.refresh += 1;
                        members.refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error(format!("Delete failed: {e}"))),
                }
            });
        }
    };

    let on_delete_cancel = move |_| {
        delete_modal.close();
    };

    // --- Attendance handlers ---

    let mut open_attendance = move |evt: Event| {
        let today = "2026-03-03".to_string();
        att_date.set(today);
        // Initialize entries from members
        let mems = members.data.read();
        let mems = mems.as_ref().and_then(|d| d.as_ref());
        if let Some(list) = mems {
            att_entries.set(
                list.iter()
                    .map(|m| AttendanceEntry {
                        member_id: m.id.clone(),
                        status: "attended".to_string(),
                    })
                    .collect(),
            );
        } else {
            att_entries.set(Vec::new());
        }
        att_modal.show(evt);
    };

    let on_att_close = move |_| {
        att_modal.close();
    };

    let on_att_submit = move |_| {
        if let Some(evt) = att_modal.get_target() {
            let payload = BatchAttendanceRequest {
                occurrence_date: att_date(),
                entries: att_entries(),
            };
            let event_id = evt.id.clone();
            att_modal.start_submit();
            spawn(async move {
                let result = ApiClient::web()
                    .post_json_empty(&format!("/api/events/{event_id}/attendance"), &payload)
                    .await;
                att_modal.end_submit();
                match result {
                    Ok(_) => {
                        toast.show(Toast::success("Attendance saved."));
                        att_modal.close();
                    }
                    Err(e) => toast.show(Toast::error(format!("Failed to save attendance: {e}"))),
                }
            });
        }
    };

    // --- Render ---

    rsx! {
        style { {ADMIN_SHARED_CSS} }

        div { class: "admin-toolbar",
            h1 { "Schedule" }
            button { class: "btn-add", onclick: open_create, "+ Add Event" }
        }

        // Events table
        {
            let data = events.data.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => rsx! { p { class: "admin-loading", "Loading..." } },
                Some(list) if list.is_empty() => rsx! {
                    p { class: "empty-state", "No events scheduled yet." }
                },
                Some(list) => rsx! {
                    DataTable { headers: vec!["Title", "Day", "Time", "Timezone", "Recurring", "Team", "Actions"],
                        for evt in list.iter() {
                            {
                                let e_edit = evt.clone();
                                let e_del = evt.clone();
                                let e_att = evt.clone();
                                let day_label = DAYS.get(evt.day_of_week as usize).unwrap_or(&"?");
                                let recurring_label = if evt.is_recurring { "Yes" } else { "No" };
                                let team_label = evt.team_name.clone().unwrap_or_else(|| "\u{2014}".into());
                                rsx! {
                                    tr { key: "{evt.id}",
                                        td { "{evt.title}" }
                                        td { "{day_label}" }
                                        td { "{evt.time}" }
                                        td { "{evt.timezone}" }
                                        td { "{recurring_label}" }
                                        td { "{team_label}" }
                                        td {
                                            div { class: "row-actions",
                                                button {
                                                    class: "row-btn",
                                                    onclick: move |_| open_edit(e_edit.clone()),
                                                    "Edit"
                                                }
                                                button {
                                                    class: "row-btn danger",
                                                    onclick: move |_| open_delete(e_del.clone()),
                                                    "Delete"
                                                }
                                                button {
                                                    class: "row-btn",
                                                    onclick: move |_| open_attendance(e_att.clone()),
                                                    "Attendance"
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

        // Create/Edit modal
        FormModal {
            title: if modal.get_target().is_some() { "Edit Event".to_string() } else { "Add Event".to_string() },
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
                label { class: "form-label", "Day of Week" }
                select {
                    class: "form-select",
                    value: "{form_day}",
                    onchange: move |e| {
                        if let Ok(v) = e.value().parse::<u8>() {
                            form_day.set(v);
                        }
                    },
                    for (i, day) in DAYS.iter().enumerate() {
                        option { value: "{i}", "{day}" }
                    }
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Time" }
                input {
                    class: "form-input",
                    r#type: "time",
                    value: "{form_time}",
                    oninput: move |e| form_time.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Timezone" }
                select {
                    class: "form-select",
                    value: "{form_tz}",
                    onchange: move |e| form_tz.set(e.value()),
                    for tz in TIMEZONES.iter() {
                        option { value: "{tz}", "{tz}" }
                    }
                }
            }
            div { class: "form-field",
                div { class: "form-checkbox-row",
                    input {
                        r#type: "checkbox",
                        checked: form_recurring(),
                        onchange: move |e| form_recurring.set(e.checked()),
                    }
                    label { class: "form-label", "Recurring" }
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Team (optional)" }
                select {
                    class: "form-select",
                    value: form_team_id().unwrap_or_default(),
                    onchange: move |e| {
                        let v = e.value();
                        form_team_id.set(if v.is_empty() { None } else { Some(v) });
                    },
                    option { value: "", "\u{2014} None \u{2014}" }
                    {
                        let teams_data = teams.data.read();
                        let teams_data = teams_data.as_ref().and_then(|d| d.as_ref());
                        match teams_data {
                            Some(list) => rsx! {
                                for t in list.iter() {
                                    option { value: "{t.id}", "{t.name}" }
                                }
                            },
                            None => rsx! {},
                        }
                    }
                }
            }
        }

        // Delete confirm
        ConfirmDialog {
            title: "Delete Event".to_string(),
            message: format!(
                "Are you sure you want to delete \"{}\"?",
                delete_modal.get_target().map(|e| e.title).unwrap_or_default()
            ),
            open: delete_modal.is_open(),
            danger: true,
            on_confirm: on_delete_confirm,
            on_cancel: on_delete_cancel,
        }

        // Attendance modal
        FormModal {
            title: format!(
                "Attendance: {}",
                att_modal.get_target().map(|e| e.title).unwrap_or_default()
            ),
            open: att_modal.is_open(),
            submitting: att_modal.is_submitting(),
            on_close: on_att_close,
            on_submit: on_att_submit,

            div { class: "form-field",
                label { class: "form-label", "Occurrence Date" }
                input {
                    class: "form-input",
                    r#type: "date",
                    value: "{att_date}",
                    oninput: move |e| att_date.set(e.value()),
                }
            }

            {
                let mems = members.data.read();
                let mems = mems.as_ref().and_then(|d| d.as_ref());
                let entries = att_entries.read();
                match mems {
                    Some(list) if !list.is_empty() => rsx! {
                        table { class: "data-table",
                            thead {
                                tr {
                                    th { "Member" }
                                    th { "Status" }
                                }
                            }
                            tbody {
                                for (idx, member) in list.iter().enumerate() {
                                    {
                                        let current_status = entries
                                            .get(idx)
                                            .map(|e| e.status.clone())
                                            .unwrap_or_else(|| "attended".to_string());
                                        rsx! {
                                            tr { key: "{member.id}",
                                                td { "{member.display_name}" }
                                                td {
                                                    select {
                                                        class: "form-select",
                                                        value: "{current_status}",
                                                        onchange: move |e| {
                                                            let val = e.value();
                                                            let mut ents = att_entries.write();
                                                            if let Some(entry) = ents.get_mut(idx) {
                                                                entry.status = val;
                                                            }
                                                        },
                                                        option { value: "attended", "Attended" }
                                                        option { value: "absent", "Absent" }
                                                        option { value: "excused", "Excused" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    },
                    Some(_) => rsx! {
                        p { class: "empty-state", "No members found." }
                    },
                    None => rsx! {
                        p { class: "admin-loading", "Loading members..." }
                    },
                }
            }
        }
    }
}
