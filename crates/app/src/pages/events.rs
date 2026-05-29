use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::{Toast, use_toast};
use crate::hooks::CursorPage;
use crate::state::auth::use_auth;
use scuffed_api_client::ApiClient;

// --- Types ---

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Event {
    id: String,
    title: String,
    day_of_week: u8,
    time: String,
    timezone: String,
    duration_minutes: u32,
    is_recurring: bool,
    #[allow(dead_code)]
    team_id: Option<String>,
    #[allow(dead_code)]
    created_by: String,
    is_active: bool,
}

#[derive(Serialize)]
struct RsvpRequest {
    status: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RsvpSummary {
    #[allow(dead_code)]
    event_id: String,
    yes_count: u32,
    maybe_count: u32,
    no_count: u32,
}

const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

const PAGE_CSS: &str = r#"
    .events-page {
        padding: 3rem 2rem;
        max-width: 1100px;
        margin: 0 auto;
    }
    .events-page h1 {
        font-family: 'Bebas Neue', sans-serif;
        font-size: 2.5rem;
        color: var(--text-bright);
        letter-spacing: 3px;
        margin: 0 0 0.5rem;
    }
    .events-subtitle {
        color: var(--text-muted);
        font-size: 0.85rem;
        margin: 0 0 2rem;
    }
    .events-week {
        display: flex;
        flex-direction: column;
        gap: 1.5rem;
    }
    .events-day {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
    }
    .events-day-header {
        font-family: 'Rajdhani', sans-serif;
        font-weight: 700;
        font-size: 1.1rem;
        color: #a78bfa;
        text-transform: uppercase;
        letter-spacing: 0.06em;
        padding-bottom: 0.35rem;
        border-bottom: 1px solid #7c3aed33;
    }
    .events-day-cards {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
        gap: 0.75rem;
    }
    .event-card {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.25rem;
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
        transition: border-color 0.2s;
    }
    .event-card:hover {
        border-color: var(--accent-soft);
    }
    .event-card-title {
        font-family: 'Rajdhani', sans-serif;
        font-weight: 700;
        font-size: 1.1rem;
        color: var(--text-bright);
        margin: 0;
    }
    .event-card-meta {
        display: flex;
        flex-wrap: wrap;
        gap: 0.75rem;
        font-size: 0.78rem;
        color: var(--text-muted);
    }
    .event-card-meta span {
        display: flex;
        align-items: center;
        gap: 0.25rem;
    }
    .event-recurring-badge {
        display: inline-block;
        font-size: 0.6rem;
        padding: 0.1rem 0.5rem;
        border-radius: 999px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
        background: #7c3aed33;
        color: #a78bfa;
    }
    .event-rsvp-row {
        display: flex;
        gap: 0.5rem;
        align-items: center;
        flex-wrap: wrap;
        margin-top: 0.25rem;
    }
    .event-rsvp-btn {
        padding: 0.3rem 0.75rem;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: transparent;
        color: var(--text-secondary);
        font-size: 0.75rem;
        font-weight: 600;
        cursor: pointer;
        transition: background 0.15s, color 0.15s, border-color 0.15s;
    }
    .event-rsvp-btn:hover {
        background: var(--bg-card);
        color: var(--text-bright);
        border-color: var(--accent-soft);
    }
    .event-rsvp-btn.active-yes {
        background: #10b98133;
        color: #34d399;
        border-color: #10b98155;
    }
    .event-rsvp-btn.active-maybe {
        background: #f9731633;
        color: #f97316;
        border-color: #f9731655;
    }
    .event-rsvp-btn.active-no {
        background: #ef444433;
        color: #f87171;
        border-color: #ef444455;
    }
    .event-rsvp-counts {
        display: flex;
        gap: 0.75rem;
        font-size: 0.7rem;
        color: var(--text-muted);
        margin-top: 0.15rem;
    }
    .event-rsvp-counts span {
        display: flex;
        align-items: center;
        gap: 0.2rem;
    }
    .rsvp-dot {
        width: 6px;
        height: 6px;
        border-radius: 50%;
        display: inline-block;
    }
    .rsvp-dot.yes { background: #34d399; }
    .rsvp-dot.maybe { background: #f97316; }
    .rsvp-dot.no { background: #f87171; }
    .events-loading, .events-empty {
        color: var(--text-muted);
        text-align: center;
        padding: 3rem 0;
    }
    .events-no-day {
        color: var(--text-muted);
        font-size: 0.8rem;
        font-style: italic;
        padding: 0.25rem 0;
    }
    @media (max-width: 640px) {
        .events-day-cards {
            grid-template-columns: 1fr;
        }
        .events-page {
            padding: 2rem 1rem;
        }
    }
"#;

fn format_duration(minutes: u32) -> String {
    if minutes >= 60 {
        let h = minutes / 60;
        let m = minutes % 60;
        if m == 0 {
            format!("{h}h")
        } else {
            format!("{h}h {m}m")
        }
    } else {
        format!("{minutes}m")
    }
}

#[component]
pub fn Events() -> Element {
    let refresh = use_signal(|| 0u64);

    let events = use_resource(move || async move {
        let _ = refresh();
        ApiClient::web()
            .fetch::<CursorPage<Event>>("/api/events")
            .await
            .ok()
            .map(|r| r.data)
    });

    rsx! {
        style { {PAGE_CSS} }

        main { class: "events-page",
            h1 { "Events" }
            p { class: "events-subtitle", "Weekly recurring schedule" }

            {
                let data = events.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! { p { class: "events-loading", "Loading..." } },
                    Some(list) => {
                        let active: Vec<&Event> = list.iter()
                            .filter(|e| e.is_active)
                            .collect();
                        if active.is_empty() {
                            rsx! { p { class: "events-empty", "No events scheduled yet." } }
                        } else {
                            rsx! {
                                div { class: "events-week",
                                    for day_idx in 0u8..7u8 {
                                        {render_day_group(&active, day_idx, refresh)}
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn render_day_group(events: &[&Event], day: u8, refresh: Signal<u64>) -> Element {
    let day_events: Vec<Event> = events
        .iter()
        .filter(|e| e.day_of_week == day)
        .map(|e| (*e).clone())
        .collect();

    if day_events.is_empty() {
        return rsx! {};
    }

    let day_name = DAYS.get(day as usize).unwrap_or(&"Unknown");

    rsx! {
        div { class: "events-day",
            div { class: "events-day-header", "{day_name}" }
            div { class: "events-day-cards",
                for evt in day_events.iter() {
                    EventCard { event: evt.clone(), refresh: refresh }
                }
            }
        }
    }
}

#[component]
fn EventCard(event: Event, refresh: Signal<u64>) -> Element {
    let event_id = event.id.clone();
    let summary = use_resource(move || {
        let eid = event_id.clone();
        async move {
            let _ = refresh();
            ApiClient::web()
                .fetch::<RsvpSummary>(&format!("/api/events/{eid}/rsvp-summary"))
                .await
                .ok()
        }
    });

    let duration_text = format_duration(event.duration_minutes);

    rsx! {
        div { class: "event-card",
            h3 { class: "event-card-title", "{event.title}" }
            div { class: "event-card-meta",
                span { "{event.time} {event.timezone}" }
                span { "{duration_text}" }
                if event.is_recurring {
                    span { class: "event-recurring-badge", "Recurring" }
                }
            }

            // RSVP summary counts
            {
                let s = summary.read();
                let s = s.as_ref().and_then(|d| d.as_ref());
                match s {
                    Some(counts) if counts.yes_count > 0 || counts.maybe_count > 0 || counts.no_count > 0 => rsx! {
                        div { class: "event-rsvp-counts",
                            span {
                                span { class: "rsvp-dot yes" }
                                "{counts.yes_count} going"
                            }
                            span {
                                span { class: "rsvp-dot maybe" }
                                "{counts.maybe_count} maybe"
                            }
                            span {
                                span { class: "rsvp-dot no" }
                                "{counts.no_count} can't"
                            }
                        }
                    },
                    _ => rsx! {},
                }
            }

            // RSVP buttons (logged-in members only)
            {
                let auth = use_auth();
                if auth().is_logged_in() {
                    rsx! { RsvpButtons { event_id: event.id.clone(), refresh: refresh } }
                } else {
                    rsx! {}
                }
            }
        }
    }
}

#[component]
fn RsvpButtons(event_id: String, refresh: Signal<u64>) -> Element {
    let mut toast = use_toast();
    let mut selected = use_signal(|| Option::<String>::None);
    let mut submitting = use_signal(|| false);

    let sel = selected();
    let yes_class = if sel.as_deref() == Some("yes") {
        "event-rsvp-btn active-yes"
    } else {
        "event-rsvp-btn"
    };
    let maybe_class = if sel.as_deref() == Some("maybe") {
        "event-rsvp-btn active-maybe"
    } else {
        "event-rsvp-btn"
    };
    let no_class = if sel.as_deref() == Some("no") {
        "event-rsvp-btn active-no"
    } else {
        "event-rsvp-btn"
    };

    let is_submitting = submitting();

    let eid_yes = event_id.clone();
    let eid_maybe = event_id.clone();
    let eid_no = event_id.clone();

    rsx! {
        div { class: "event-rsvp-row",
            button {
                class: yes_class,
                disabled: is_submitting,
                onclick: move |_| {
                    let eid = eid_yes.clone();
                    selected.set(Some("yes".to_string()));
                    submitting.set(true);
                    spawn(async move {
                        let body = RsvpRequest { status: "yes".to_string() };
                        match ApiClient::web()
                            .post_json::<_, serde_json::Value>(&format!("/api/events/{eid}/rsvp"), &body)
                            .await
                        {
                            Ok(_) => {
                                toast.show(Toast::success("RSVP: Going".to_string()));
                                refresh += 1;
                            }
                            Err(e) => {
                                toast.show(Toast::error(format!("RSVP failed: {e}")));
                                selected.set(None);
                            }
                        }
                        submitting.set(false);
                    });
                },
                "Going"
            }
            button {
                class: maybe_class,
                disabled: is_submitting,
                onclick: move |_| {
                    let eid = eid_maybe.clone();
                    selected.set(Some("maybe".to_string()));
                    submitting.set(true);
                    spawn(async move {
                        let body = RsvpRequest { status: "maybe".to_string() };
                        match ApiClient::web()
                            .post_json::<_, serde_json::Value>(&format!("/api/events/{eid}/rsvp"), &body)
                            .await
                        {
                            Ok(_) => {
                                toast.show(Toast::success("RSVP: Maybe".to_string()));
                                refresh += 1;
                            }
                            Err(e) => {
                                toast.show(Toast::error(format!("RSVP failed: {e}")));
                                selected.set(None);
                            }
                        }
                        submitting.set(false);
                    });
                },
                "Maybe"
            }
            button {
                class: no_class,
                disabled: is_submitting,
                onclick: move |_| {
                    let eid = eid_no.clone();
                    selected.set(Some("no".to_string()));
                    submitting.set(true);
                    spawn(async move {
                        let body = RsvpRequest { status: "no".to_string() };
                        match ApiClient::web()
                            .post_json::<_, serde_json::Value>(&format!("/api/events/{eid}/rsvp"), &body)
                            .await
                        {
                            Ok(_) => {
                                toast.show(Toast::success("RSVP: Can't make it".to_string()));
                                refresh += 1;
                            }
                            Err(e) => {
                                toast.show(Toast::error(format!("RSVP failed: {e}")));
                                selected.set(None);
                            }
                        }
                        submitting.set(false);
                    });
                },
                "Can't"
            }
        }
    }
}
