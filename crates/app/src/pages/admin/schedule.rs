use dioxus::prelude::*;

use crate::components::admin_shared::{after_delete_focus_ids, focus_after_delete_soon};
use serde::Deserialize;

use crate::components::ui::{
    EmptyCard, FieldError, PageHeader, PageShell, PageWidth, Pager, Pill, PillTone, page_slice,
    show_list_top,
};
use crate::components::{
    ADMIN_PAGE_SIZE, ConfirmDialog, DataTable, FormModal, RowActions, SaveState, Toast,
    admin_list_pending, admin_pending, list_cap_notice, use_toast,
};
use crate::hooks::{ModalController, use_api_list, use_api_with};
use crate::routes::Route;
use scuffed_api_client::ApiClient;
use scuffed_types::api::{AttendanceEntry, BatchAttendanceRequest, CreateEventRequest};

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
    #[serde(default)]
    is_public: bool,
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

#[derive(Debug, Clone, Deserialize)]
struct RsvpSummary {
    #[allow(dead_code)]
    event_id: String,
    yes_count: u32,
    maybe_count: u32,
    no_count: u32,
}

/// "3 yes · 1 maybe · 0 no": the RSVP counts in words, never colour alone.
fn rsvp_text(s: &RsvpSummary) -> String {
    format!(
        "{} yes \u{b7} {} maybe \u{b7} {} no",
        s.yes_count, s.maybe_count, s.no_count
    )
}

/// Inline RSVP summary cell for the schedule table.
#[component]
fn RsvpCell(event_id: String) -> Element {
    let summary =
        use_api_with::<RsvpSummary>(move || format!("/api/events/{}/rsvp-summary", event_id));

    let failed = summary.error.read().is_some();
    let data = summary.data.read();
    let data = data.as_ref().and_then(|d| d.as_ref());
    match data {
        Some(s) => {
            let text = rsvp_text(s);
            rsx! { span { class: "schedule-rsvp", "{text}" } }
        }
        None if failed => rsx! { span { class: "schedule-rsvp is-muted", "Not available" } },
        None => rsx! { span { class: "schedule-rsvp is-muted", "Loading..." } },
    }
}

const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const TIMEZONES: [&str; 5] = [
    "UTC",
    "Europe/London",
    "Europe/Berlin",
    "US/Eastern",
    "US/Pacific",
];

/// The list region: the pager's focus target after a page change.
const LIST_ID: &str = "admin-schedule-list";
const NEW_EVENT_ID: &str = "schedule-new";
const TITLE_ID: &str = "event-title";
const TITLE_ERROR_ID: &str = "event-title-error";
const DAY_ID: &str = "event-day";
const TIME_ID: &str = "event-time";
const TZ_ID: &str = "event-timezone";
const RECURRING_ID: &str = "event-recurring";
const PUBLIC_ID: &str = "event-public";
const TEAM_ID: &str = "event-team";
const ATT_DATE_ID: &str = "attendance-date";

/// Page CSS: the RSVP line keeps to one line on desktop.
const PAGE_CSS: &str = r#"
.admin-schedule-page .schedule-rsvp { white-space: nowrap; }
.admin-schedule-page .schedule-rsvp.is-muted { color: var(--text-3); }
.admin-schedule-page .schedule-pills {
    display: inline-flex;
    flex-wrap: wrap;
    gap: var(--space-2);
}
"#;

/// "Mon 19:00 UTC": when a weekly event happens.
fn when_text(day_of_week: u8, time: &str, timezone: &str) -> String {
    let day = DAYS
        .get(day_of_week as usize)
        .copied()
        .unwrap_or("Unknown day");
    format!("{day} {time} {timezone}")
}

/// Id of a row's Edit button, from the event id. Dialogs opened from the
/// row return focus here, because it is on screen at every width.
fn event_edit_button_id(event_id: &str) -> String {
    format!("event-edit-{event_id}")
}

/// The occurrence the server stores attendance under: the picked day at
/// midnight UTC, as an RFC 3339 timestamp (the server reads a `DateTime`, and
/// a bare `2026-11-17` never parsed). `None` for anything that is not a
/// `YYYY-MM-DD` day.
fn occurrence_timestamp(day: &str) -> Option<String> {
    let day = chrono::NaiveDate::parse_from_str(day.trim(), "%Y-%m-%d").ok()?;
    Some(format!("{}T00:00:00Z", day.format("%Y-%m-%d")))
}

/// Id of an attendance row's select, from the member id.
fn attendance_select_id(member_id: &str) -> String {
    format!("attendance-{member_id}")
}

#[component]
pub fn AdminSchedule() -> Element {
    let mut events = use_api_list::<Event>("/api/events");
    let mut teams = use_api_list::<Team>("/api/teams");
    let mut members = use_api_list::<Member>("/api/members");
    let mut toast = use_toast();
    let mut page = use_signal(|| 1usize);

    // Form modal state
    let mut modal = ModalController::<String>::new();
    let mut form_title = use_signal(String::new);
    let mut form_day = use_signal(|| 0u8);
    let mut form_time = use_signal(|| "19:00".to_string());
    let mut form_tz = use_signal(|| "UTC".to_string());
    let mut form_recurring = use_signal(|| true);
    let mut form_public = use_signal(|| false);
    let mut form_team_id: Signal<Option<String>> = use_signal(|| None);
    let mut title_error = use_signal(String::new);
    let mut save_error = use_signal(|| None::<String>);
    let mut opener = use_signal(|| NEW_EVENT_ID.to_string());

    // Delete confirm state
    let mut delete_modal = ModalController::<Event>::new();

    // Attendance modal state
    let mut att_modal = ModalController::<Event>::new();
    let mut att_date = use_signal(String::new);
    let mut att_entries: Signal<Vec<AttendanceEntry>> = use_signal(Vec::new);
    let mut att_error = use_signal(|| None::<String>);

    // --- Event form handlers ---

    let mut open_create = move || {
        form_title.set(String::new());
        form_day.set(0);
        form_time.set("19:00".to_string());
        form_tz.set("UTC".to_string());
        form_recurring.set(true);
        form_public.set(false);
        form_team_id.set(None);
        title_error.set(String::new());
        save_error.set(None);
        opener.set(NEW_EVENT_ID.to_string());
        modal.show_empty();
    };

    let mut open_edit = move |evt: Event| {
        opener.set(event_edit_button_id(&evt.id));
        form_title.set(evt.title);
        form_day.set(evt.day_of_week);
        form_time.set(evt.time);
        form_tz.set(evt.timezone);
        form_recurring.set(evt.is_recurring);
        form_public.set(evt.is_public);
        form_team_id.set(evt.team_id);
        title_error.set(String::new());
        save_error.set(None);
        modal.show(evt.id);
    };

    let on_close = move |_| modal.close();

    let on_submit = move |_| {
        let title = form_title().trim().to_string();
        save_error.set(None);
        if title.is_empty() {
            title_error.set("Enter a title for the event.".to_string());
            return;
        }
        title_error.set(String::new());
        let body = CreateEventRequest {
            title,
            day_of_week: form_day(),
            time: form_time().trim().to_string(),
            timezone: form_tz(),
            is_recurring: form_recurring(),
            team_id: form_team_id(),
            is_public: form_public(),
        };
        let edit_id = modal.get_target();
        modal.start_submit();
        spawn(async move {
            let client = ApiClient::web();
            let result = if let Some(id) = edit_id {
                client
                    .put_json::<_, Event>(&format!("/api/events/{id}"), &body)
                    .await
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
                Err(e) => {
                    save_error.set(Some(
                        SaveState::failed("Could not save the event", &e).text(),
                    ));
                    toast.show(Toast::error_logged_elsewhere("Failed to save event", &e));
                }
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
            // B9: once the row is gone, focus the next row's Edit, else the
            // previous row's, else New event.
            let keys: Vec<String> = events
                .data
                .peek()
                .as_ref()
                .and_then(|d| d.as_ref())
                .map(|all| all.iter().map(|e| e.id.clone()).collect())
                .unwrap_or_default();
            let focus_next = after_delete_focus_ids(
                &keys,
                &id,
                |k| vec![event_edit_button_id(k)],
                &[NEW_EVENT_ID],
            );
            delete_modal.close();
            spawn(async move {
                match ApiClient::web().delete(&format!("/api/events/{id}")).await {
                    Ok(_) => {
                        toast.show(Toast::success("Event deleted."));
                        focus_after_delete_soon(event_edit_button_id(&id), focus_next);
                        events.refresh += 1;
                        teams.refresh += 1;
                        members.refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error_from("Delete failed", &e)),
                }
            });
        }
    };

    let on_delete_cancel = move |_| {
        delete_modal.close();
    };

    // --- Attendance handlers ---

    let mut open_attendance = move |evt: Event| {
        // Local calendar date YYYY-MM-DD (was hardcoded 2026-03-03)
        let today = {
            use js_sys::Date;
            let d = Date::new_0();
            format!(
                "{:04}-{:02}-{:02}",
                d.get_full_year() as i32,
                d.get_month() + 1,
                { d.get_date() }
            )
        };
        att_date.set(today);
        att_error.set(None);
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
            let Some(occurrence_date) = occurrence_timestamp(&att_date()) else {
                att_error.set(Some("Pick the date of the event.".to_string()));
                return;
            };
            let payload = BatchAttendanceRequest {
                occurrence_date,
                entries: att_entries(),
            };
            let event_id = evt.id.clone();
            att_error.set(None);
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
                    Err(e) => {
                        att_error.set(Some(
                            SaveState::failed("Could not save attendance", &e).text(),
                        ));
                        toast.show(Toast::error_logged_elsewhere(
                            "Failed to save attendance",
                            &e,
                        ));
                    }
                }
            });
        }
    };

    // --- Render ---

    let list = {
        let data = events.data.read();
        match data.as_ref().and_then(|d| d.as_ref()) {
            None => admin_list_pending(&events, "events"),
            Some(all) => {
                let total = all.len();
                let rows = page_slice(all, page(), ADMIN_PAGE_SIZE).to_vec();
                rsx! {
                    if all.is_empty() {
                        EmptyCard {
                            title: "No events yet",
                            message: "Add the weekly practice and match times, so members can RSVP and you can take attendance.",
                            actions: rsx! {
                                button {
                                    r#type: "button",
                                    class: "btn is-primary",
                                    onclick: move |_| open_create(),
                                    "New event"
                                }
                            },
                        }
                    } else {
                        DataTable {
                            headers: vec!["Event", "Visibility", "When", "Team", "RSVP", "Actions"],
                            cards: true,
                            label: "Events",
                            for evt in rows.iter() {
                                Fragment { key: "{evt.id}", {
                                    let e_edit = evt.clone();
                                    let e_del = evt.clone();
                                    let e_att = evt.clone();
                                    let rsvp_id = evt.id.clone();
                                    let title = evt.title.clone();
                                    let when = when_text(evt.day_of_week, &evt.time, &evt.timezone);
                                    let team_label = evt.team_name.clone().unwrap_or_else(|| "All members".into());
                                    let more = rsx! {
                                        button {
                                            r#type: "button",
                                            class: "btn is-ghost",
                                            aria_label: "Attendance for {title}",
                                            onclick: move |_| open_attendance(e_att.clone()),
                                            "Attendance"
                                        }
                                        button {
                                            r#type: "button",
                                            class: "btn is-danger",
                                            aria_label: "Delete {title}",
                                            onclick: move |_| open_delete(e_del.clone()),
                                            "Delete"
                                        }
                                    };
                                    rsx! {
                                        tr { key: "{evt.id}",
                                            td { class: "cell-who", "{evt.title}" }
                                            td { class: "cell-badge",
                                                span { class: "schedule-pills",
                                                    if evt.is_public {
                                                        Pill { tone: PillTone::Info, "Public" }
                                                    } else {
                                                        Pill { tone: PillTone::Neutral, "Members only" }
                                                    }
                                                    if evt.is_recurring {
                                                        Pill { tone: PillTone::Neutral, "Weekly" }
                                                    } else {
                                                        Pill { tone: PillTone::Neutral, "One time" }
                                                    }
                                                }
                                            }
                                            td { class: "cell-meta", "{when}" }
                                            td { class: "cell-meta", "{team_label}" }
                                            td { "data-label": "RSVP", RsvpCell { event_id: rsvp_id } }
                                            td { class: "cell-actions",
                                                RowActions {
                                                    id: "event-actions-{evt.id}",
                                                    label: "Actions for {title}",
                                                    inline_on_desktop: true,
                                                    more,
                                                    button {
                                                        r#type: "button",
                                                        class: "btn is-ghost",
                                                        id: event_edit_button_id(&evt.id),
                                                        aria_label: "Edit {title}",
                                                        onclick: move |_| open_edit(e_edit.clone()),
                                                        "Edit"
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
                        id: "schedule-pager",
                        page: page(),
                        total,
                        page_size: ADMIN_PAGE_SIZE,
                        noun: "events",
                        noun_singular: "event",
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

    let delete_focus = delete_modal
        .get_target()
        .map(|e| event_edit_button_id(&e.id));
    let att_focus = att_modal.get_target().map(|e| event_edit_button_id(&e.id));

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-admin-schedule-page-css", css: PAGE_CSS }
        PageShell { width: PageWidth::Wide, class: "admin-schedule-page",
            PageHeader {
                title: "Schedule",
                lead: "Weekly practice and match times. Public events show on the site and in the calendar feed.",
                actions: rsx! {
                    button {
                        r#type: "button",
                        class: "btn is-primary",
                        id: NEW_EVENT_ID,
                        onclick: move |_| open_create(),
                        "New event"
                    }
                    Link { to: Route::Events {}, class: "btn is-ghost", "View public events" }
                },
            }

            section {
                id: LIST_ID,
                class: "admin-list pager-target",
                tabindex: "-1",
                aria_label: "Events",
                {list}
            }

            {list_cap_notice(&events, "events")}
            {list_cap_notice(&teams, "teams")}
            {list_cap_notice(&members, "members")}
        }

        // Create/Edit modal
        FormModal {
            title: if modal.get_target().is_some() { "Edit event".to_string() } else { "New event".to_string() },
            open: modal.is_open(),
            submitting: modal.is_submitting(),
            on_close: on_close,
            on_submit: on_submit,
            submit_label: modal.get_target().is_none().then(|| "Create".to_string()),
            error: save_error(),
            restore_focus_id: Some(opener()),

            div { class: "form-field",
                label { class: "field-label", r#for: TITLE_ID, "Title" }
                input {
                    class: "field-input",
                    id: TITLE_ID,
                    r#type: "text",
                    value: "{form_title}",
                    aria_invalid: if !title_error().is_empty() { "true" },
                    aria_describedby: TITLE_ERROR_ID,
                    oninput: move |e| form_title.set(e.value()),
                }
                FieldError { id: TITLE_ERROR_ID.to_string(), text: title_error() }
            }
            div { class: "form-grid",
                div { class: "form-field",
                    label { class: "field-label", r#for: DAY_ID, "Day of the week" }
                    select {
                        class: "field-input",
                        id: DAY_ID,
                        value: "{form_day}",
                        onchange: move |e| {
                            if let Ok(v) = e.value().parse::<u8>() {
                                form_day.set(v);
                            }
                        },
                        for (i, day) in DAYS.iter().enumerate() {
                            option { key: "{i}", value: "{i}", "{day}" }
                        }
                    }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: TIME_ID, "Time" }
                    input {
                        class: "field-input",
                        id: TIME_ID,
                        r#type: "time",
                        value: "{form_time}",
                        oninput: move |e| form_time.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: TZ_ID, "Timezone" }
                    select {
                        class: "field-input",
                        id: TZ_ID,
                        value: "{form_tz}",
                        onchange: move |e| form_tz.set(e.value()),
                        for tz in TIMEZONES.iter() {
                            option { key: "{tz}", value: "{tz}", "{tz}" }
                        }
                    }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: TEAM_ID, "Team (optional)" }
                    select {
                        class: "field-input",
                        id: TEAM_ID,
                        value: form_team_id().unwrap_or_default(),
                        onchange: move |e| {
                            let v = e.value();
                            form_team_id.set(if v.is_empty() { None } else { Some(v) });
                        },
                        option { value: "", "All members" }
                        {
                            let teams_data = teams.data.read();
                            let teams_data = teams_data.as_ref().and_then(|d| d.as_ref());
                            match teams_data {
                                Some(list) => rsx! {
                                    for t in list.iter() {
                                        option { key: "{t.id}", value: "{t.id}", "{t.name}" }
                                    }
                                },
                                None => rsx! {},
                            }
                        }
                    }
                }
            }
            label { class: "admin-check-row", r#for: RECURRING_ID,
                input {
                    r#type: "checkbox",
                    id: RECURRING_ID,
                    checked: form_recurring(),
                    onchange: move |e| form_recurring.set(e.checked()),
                }
                "Repeats every week"
            }
            label { class: "admin-check-row", r#for: PUBLIC_ID,
                input {
                    r#type: "checkbox",
                    id: PUBLIC_ID,
                    checked: form_public(),
                    onchange: move |e| form_public.set(e.checked()),
                }
                "Public (show on the site and in the calendar feed)"
            }
        }

        // Delete confirm
        ConfirmDialog {
            title: "Delete event".to_string(),
            message: format!(
                "Are you sure you want to delete \"{}\"?",
                delete_modal.get_target().map(|e| e.title).unwrap_or_default()
            ),
            open: delete_modal.is_open(),
            danger: true,
            confirm_label: "Delete".to_string(),
            restore_focus_id: delete_focus,
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
            wide: true,
            submit_label: Some("Save attendance".to_string()),
            error: att_error(),
            restore_focus_id: att_focus,

            div { class: "form-field",
                label { class: "field-label", r#for: ATT_DATE_ID, "Date of the event" }
                input {
                    class: "field-input",
                    id: ATT_DATE_ID,
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
                        DataTable {
                            headers: vec!["Member", "Attendance"],
                            cards: true,
                            label: "Attendance",
                            for (idx, member) in list.iter().enumerate() {
                                Fragment { key: "{member.id}", {
                                    let current_status = entries
                                        .get(idx)
                                        .map(|e| e.status.clone())
                                        .unwrap_or_else(|| "attended".to_string());
                                    let name = member.display_name.clone();
                                    rsx! {
                                        tr { key: "{member.id}",
                                            td { class: "cell-who", "{member.display_name}" }
                                            td { "data-label": "Attendance",
                                                select {
                                                    class: "field-input",
                                                    id: attendance_select_id(&member.id),
                                                    aria_label: "Attendance of {name}",
                                                    value: "{current_status}",
                                                    onchange: move |e| {
                                                        let val = e.value();
                                                        let mut ents = att_entries.write();
                                                        if let Some(entry) = ents.get_mut(idx) {
                                                            entry.status = val;
                                                        }
                                                    },
                                                    option { value: "attended", "Attended" }
                                                    // The server's word is `no_show`; "absent"
                                                    // was rejected, so nothing saved.
                                                    option { value: "no_show", "Absent" }
                                                    option { value: "excused", "Excused" }
                                                }
                                            }
                                        }
                                    }
                                } }
                            }
                        }
                    },
                    Some(_) => rsx! {
                        p { class: "field-hint", "No members to mark yet. Members appear here once they join." }
                    },
                    None => admin_pending(&members, "members"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This sheet sets no type, so the shared check (which needs a
    /// font-size) does not apply: no rem, no px past a hairline, no hex.
    fn assert_page_css_on_tokens(css: &str) {
        for word in css.split(|c: char| c.is_whitespace() || ";:(),".contains(c)) {
            assert!(!word.ends_with("rem"), "{word}");
            assert!(!word.ends_with("px") || word == "1px", "{word}");
            assert!(!word.starts_with('#'), "{word}");
        }
        assert!(
            !css.contains("font-size"),
            "type goes through assert_sizes_from_tokens"
        );
    }
    use crate::components::admin_shared::test_page::render_admin_route;
    use scuffed_types::OrgRole;

    #[test]
    fn attendance_sends_a_timestamp_and_the_servers_status_words() {
        // Vision nit 14 on PR 706: a bare date and "absent" were both
        // rejected, so attendance could never save.
        assert_eq!(
            occurrence_timestamp("2026-11-17").as_deref(),
            Some("2026-11-17T00:00:00Z")
        );
        assert!(
            chrono::DateTime::parse_from_rfc3339(&occurrence_timestamp("2026-11-17").unwrap())
                .is_ok()
        );
        assert_eq!(occurrence_timestamp(""), None);
        assert_eq!(occurrence_timestamp("17/11/2026"), None);
        let src = include_str!("schedule.rs");
        let prod = src.split("mod tests").next().expect("production half");
        assert!(prod.contains("option { value: \"no_show\", \"Absent\" }"));
        assert!(!prod.contains("value: \"absent\""));
        for word in ["\"attended\"", "\"no_show\"", "\"excused\""] {
            let status: scuffed_types::AttendanceStatus = serde_json::from_str(word).expect(word);
            let _ = status;
        }
        assert!(serde_json::from_str::<scuffed_types::AttendanceStatus>("\"absent\"").is_err());
    }

    #[test]
    fn page_has_one_h1_and_a_labelled_wait_in_the_wide_shell() {
        let html = render_admin_route("/admin/schedule", Some(OrgRole::Officer));
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide admin-schedule-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Schedule</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(
            html.contains(
                "<section id=\"admin-schedule-list\" class=\"admin-list pager-target\" tabindex=\"-1\" aria-label=\"Events\">"
            ),
            "{html}"
        );
        assert!(
            html.contains("role=\"status\">Loading events...</p>"),
            "{html}"
        );
    }

    #[test]
    fn rsvp_and_when_are_words() {
        let s = RsvpSummary {
            event_id: "e1".into(),
            yes_count: 3,
            maybe_count: 1,
            no_count: 0,
        };
        assert_eq!(rsvp_text(&s), "3 yes \u{b7} 1 maybe \u{b7} 0 no");
        assert_eq!(when_text(0, "19:00", "UTC"), "Mon 19:00 UTC");
        assert_eq!(when_text(9, "19:00", "UTC"), "Unknown day 19:00 UTC");
        assert_page_css_on_tokens(PAGE_CSS);
    }

    #[test]
    fn every_field_has_a_tied_label_and_no_inline_style() {
        let src = include_str!("schedule.rs");
        let prod = src.split("mod tests").next().unwrap();
        for id in [
            "TITLE_ID",
            "DAY_ID",
            "TIME_ID",
            "TZ_ID",
            "TEAM_ID",
            "RECURRING_ID",
            "PUBLIC_ID",
            "ATT_DATE_ID",
        ] {
            assert!(prod.contains(&format!("r#for: {id}")), "{id}");
            assert!(prod.contains(&format!("id: {id},")), "{id}");
        }
        assert!(!prod.contains("style:"));
        assert!(!prod.contains('\u{2014}'));
        assert_eq!(attendance_select_id("m1"), "attendance-m1");
    }
}
