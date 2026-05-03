use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::{Toast, use_toast};
use crate::state::auth::use_auth;
use scuffed_api_client::ApiClient;

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Event {
    id: String,
    title: String,
    day_of_week: u8,
    time: String,
    timezone: String,
    duration_minutes: u32,
    is_recurring: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RsvpSummary {
    #[allow(dead_code)]
    event_id: String,
    yes_count: u32,
    maybe_count: u32,
    no_count: u32,
}

#[derive(Debug, Clone, Serialize)]
struct RsvpRequest {
    status: String,
}

const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

const PAGE_CSS: &str = r#"
    .events-page {
        padding: 3rem 2rem;
        max-width: 1100px;
        margin: 0 auto;
    }
    .events-page-title {
        font-family: 'Bebas Neue', sans-serif;
        font-size: 2.5rem;
        color: var(--text-bright);
        letter-spacing: 3px;
        margin: 0 0 0.5rem;
    }
    .events-subtitle {
        color: var(--text-secondary);
        font-size: 0.9rem;
        margin: 0 0 2rem;
        line-height: 1.6;
    }
    .events-week {
        display: grid;
        grid-template-columns: repeat(7, 1fr);
        gap: 0.75rem;
    }
    @media (max-width: 900px) {
        .events-week {
            grid-template-columns: repeat(2, 1fr);
        }
    }
    @media (max-width: 480px) {
        .events-week {
            grid-template-columns: 1fr;
        }
    }
    .events-day {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
    }
    .events-day-label {
        font-family: 'Rajdhani', sans-serif;
        font-weight: 700;
        font-size: 0.75rem;
        text-transform: uppercase;
        letter-spacing: 0.08em;
        color: var(--text-muted);
        padding-bottom: 0.35rem;
        border-bottom: 1px solid var(--border);
    }
    .events-day-label.has-events {
        color: var(--accent);
        border-color: var(--accent-soft);
    }
    .event-card {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1rem;
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
        transition: border-color 0.2s;
    }
    .event-card:hover {
        border-color: var(--accent-soft);
    }
    .event-title {
        font-family: 'Rajdhani', sans-serif;
        font-weight: 700;
        font-size: 1rem;
        color: var(--text-bright);
        margin: 0;
    }
    .event-meta {
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
        font-size: 0.75rem;
        color: var(--text-secondary);
    }
    .event-meta-row {
        display: flex;
        align-items: center;
        gap: 0.4rem;
    }
    .event-recurring-pill {
        display: inline-block;
        font-size: 0.6rem;
        padding: 0.1rem 0.5rem;
        border-radius: 999px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
        background: #7c3aed22;
        color: #a78bfa;
        width: fit-content;
    }
    .event-rsvp-summary {
        display: flex;
        gap: 0.6rem;
        font-size: 0.7rem;
        color: var(--text-muted);
        padding-top: 0.35rem;
        border-top: 1px solid var(--border);
    }
    .rsvp-count {
        display: flex;
        align-items: center;
        gap: 0.2rem;
    }
    .rsvp-count.yes { color: #34d399; }
    .rsvp-count.maybe { color: #fbbf24; }
    .rsvp-count.no { color: #f87171; }
    .event-rsvp-actions {
        display: flex;
        gap: 0.35rem;
    }
    .rsvp-btn {
        flex: 1;
        padding: 0.35rem 0;
        border: 1px solid var(--border);
        border-radius: 6px;
        background: transparent;
        color: var(--text-secondary);
        font-size: 0.65rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
        cursor: pointer;
        transition: background 0.15s, color 0.15s, border-color 0.15s;
    }
    .rsvp-btn:hover {
        color: var(--text-bright);
        border-color: var(--accent-soft);
    }
    .rsvp-btn.active-yes {
        background: #10b98133;
        border-color: #34d399;
        color: #34d399;
    }
    .rsvp-btn.active-maybe {
        background: #f59e0b22;
        border-color: #fbbf24;
        color: #fbbf24;
    }
    .rsvp-btn.active-no {
        background: #ef444422;
        border-color: #f87171;
        color: #f87171;
    }
    .events-day-empty {
        color: var(--text-muted);
        font-size: 0.75rem;
        font-style: italic;
        padding: 0.5rem 0;
    }
    .events-loading, .events-empty {
        color: var(--text-muted);
        text-align: center;
        padding: 3rem 0;
    }
"#;

#[component]
pub fn Events() -> Element {
    let events = use_resource(|| async {
        ApiClient::web()
            .fetch::<Vec<Event>>("/api/events")
            .await
            .ok()
    });

    rsx! {
        style { {PAGE_CSS} }

        main { class: "events-page",
            h1 { class: "events-page-title", "Events" }
            p { class: "events-subtitle",
                "Our weekly schedule. RSVP to let the crew know you\u{2019}re showing up."
            }

            {
                let data = events.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! { p { class: "events-loading", "Loading..." } },
                    Some(list) if list.is_empty() => rsx! {
                        p { class: "events-empty", "No events scheduled yet." }
                    },
                    Some(list) => rsx! {
                        div { class: "events-week",
                            for (day_idx, day_name) in DAYS.iter().enumerate() {
                                {
                                    let day_events: Vec<&Event> = list.iter()
                                        .filter(|e| e.day_of_week == day_idx as u8)
                                        .collect();
                                    let has_events = !day_events.is_empty();
                                    let label_class = if has_events {
                                        "events-day-label has-events"
                                    } else {
                                        "events-day-label"
                                    };

                                    rsx! {
                                        div { class: "events-day",
                                            div { class: "{label_class}", "{day_name}" }
                                            if day_events.is_empty() {
                                                div { class: "events-day-empty", "\u{2014}" }
                                            }
                                            for event in day_events.iter() {
                                                EventCard {
                                                    key: "{event.id}",
                                                    event: (*event).clone(),
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
        }
    }
}

#[component]
fn EventCard(event: Event) -> Element {
    let auth = use_auth();
    let mut toast = use_toast();
    let is_member = auth().user.as_ref().and_then(|u| u.role).is_some();

    let event_id = event.id.clone();
    let summary = use_resource(move || {
        let eid = event_id.clone();
        async move {
            ApiClient::web()
                .fetch::<RsvpSummary>(&format!("/api/events/{eid}/rsvp-summary"))
                .await
                .ok()
        }
    });

    let duration_text = if event.duration_minutes >= 60 {
        let h = event.duration_minutes / 60;
        let m = event.duration_minutes % 60;
        if m == 0 {
            format!("{h}h")
        } else {
            format!("{h}h {m}m")
        }
    } else {
        format!("{}m", event.duration_minutes)
    };

    let s = summary.read();
    let s = s.as_ref().and_then(|d| d.as_ref());
    let yes = s.map(|s| s.yes_count).unwrap_or(0);
    let maybe = s.map(|s| s.maybe_count).unwrap_or(0);
    let no = s.map(|s| s.no_count).unwrap_or(0);

    rsx! {
        div { class: "event-card",
            h3 { class: "event-title", "{event.title}" }
            div { class: "event-meta",
                div { class: "event-meta-row",
                    span { "{event.time}" }
                    span { "{event.timezone}" }
                    span { "{duration_text}" }
                }
                if event.is_recurring {
                    span { class: "event-recurring-pill", "Weekly" }
                }
            }

            div { class: "event-rsvp-summary",
                span { class: "rsvp-count yes", "Going {yes}" }
                span { class: "rsvp-count maybe", "Maybe {maybe}" }
                span { class: "rsvp-count no", "Can\u{2019}t {no}" }
            }

            if is_member {
                {
                    let eid_yes = event.id.clone();
                    let eid_maybe = event.id.clone();
                    let eid_no = event.id.clone();
                    let mut summary_ref = summary;

                    rsx! {
                        div { class: "event-rsvp-actions",
                            button {
                                class: "rsvp-btn",
                                onclick: move |_| {
                                    let eid = eid_yes.clone();
                                    spawn(async move {
                                        let body = RsvpRequest { status: "yes".to_string() };
                                        match ApiClient::web()
                                            .post_json_empty(&format!("/api/events/{eid}/rsvp"), &body)
                                            .await
                                        {
                                            Ok(_) => {
                                                toast.show(Toast::success("RSVP'd as Going!"));
                                                summary_ref.restart();
                                            }
                                            Err(e) => toast.show(Toast::error(format!("RSVP failed: {e}"))),
                                        }
                                    });
                                },
                                "Going"
                            }
                            button {
                                class: "rsvp-btn",
                                onclick: move |_| {
                                    let eid = eid_maybe.clone();
                                    spawn(async move {
                                        let body = RsvpRequest { status: "maybe".to_string() };
                                        match ApiClient::web()
                                            .post_json_empty(&format!("/api/events/{eid}/rsvp"), &body)
                                            .await
                                        {
                                            Ok(_) => {
                                                toast.show(Toast::success("RSVP'd as Maybe."));
                                                summary_ref.restart();
                                            }
                                            Err(e) => toast.show(Toast::error(format!("RSVP failed: {e}"))),
                                        }
                                    });
                                },
                                "Maybe"
                            }
                            button {
                                class: "rsvp-btn",
                                onclick: move |_| {
                                    let eid = eid_no.clone();
                                    spawn(async move {
                                        let body = RsvpRequest { status: "no".to_string() };
                                        match ApiClient::web()
                                            .post_json_empty(&format!("/api/events/{eid}/rsvp"), &body)
                                            .await
                                        {
                                            Ok(_) => {
                                                toast.show(Toast::success("Marked as Can't make it."));
                                                summary_ref.restart();
                                            }
                                            Err(e) => toast.show(Toast::error(format!("RSVP failed: {e}"))),
                                        }
                                    });
                                },
                                "Can\u{2019}t"
                            }
                        }
                    }
                }
            }
        }
    }
}
