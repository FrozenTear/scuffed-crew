use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};

use scuffed_auth::client::api::{fetch_json, fetch_json_list, post_json};

use crate::app::use_site_auth;
use crate::components::SectionHeader;

#[derive(Debug, Clone, Deserialize)]
struct Event {
    id: String,
    title: String,
    day_of_week: u8,
    time: String,
    timezone: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RsvpSummary {
    #[allow(dead_code)]
    event_id: String,
    yes_count: u32,
    maybe_count: u32,
    no_count: u32,
}

#[derive(Serialize)]
struct RsvpRequest {
    status: String,
}

const DAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

#[component]
pub fn Schedule() -> impl IntoView {
    let events = LocalResource::new(|| async {
        fetch_json_list::<Event>("/api/events").await.ok()
    });

    view! {
        <section id="schedule">
            <SectionHeader
                label="// Weekly Rhythm"
                title="Play Nights"
                color="purple"
                description="No obligation to hit every session. Show up when life allows. These are the nights people are online and playing."
            />

            <div class="sched-strip" data-reveal="">
                {move || {
                    let event_list = events.get().flatten().unwrap_or_default();

                    (0u8..7).map(|day| {
                        let day_events: Vec<Event> = event_list
                            .iter()
                            .filter(|e| e.day_of_week == day)
                            .cloned()
                            .collect();

                        let day_name = DAY_NAMES[day as usize];

                        if let Some(event) = day_events.first() {
                            let time_display = format!("{} {}", event.time, event.timezone);
                            let event_id = event.id.clone();
                            view! {
                                <div class="sched-active">
                                    <div class="day-label">{day_name}</div>
                                    <div class="day-event">{event.title.clone()}</div>
                                    <div class="day-time">{time_display}</div>
                                    <EventRsvp event_id=event_id/>
                                </div>
                            }.into_any()
                        } else {
                            view! {
                                <div class="sched-off">{day_name}</div>
                            }.into_any()
                        }
                    }).collect_view()
                }}
            </div>

            <div class="sched-calendar-link" data-reveal="">
                <a href="/api/calendar/all.ics" class="btn btn-outline">
                    <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <rect x="3" y="4" width="18" height="18" rx="2" ry="2"></rect>
                        <line x1="16" y1="2" x2="16" y2="6"></line>
                        <line x1="8" y1="2" x2="8" y2="6"></line>
                        <line x1="3" y1="10" x2="21" y2="10"></line>
                    </svg>
                    "Subscribe to Calendar"
                </a>
            </div>
        </section>
    }
}

/// RSVP counts + buttons for a single event in the schedule strip.
#[component]
fn EventRsvp(event_id: String) -> impl IntoView {
    let auth = use_site_auth();
    let refresh = RwSignal::new(0u32);
    let eid = StoredValue::new(event_id);

    let summary = LocalResource::new(move || {
        refresh.get();
        let eid = eid.get_value();
        async move {
            fetch_json::<RsvpSummary>(&format!("/api/events/{eid}/rsvp-summary"))
                .await
                .ok()
        }
    });

    let do_rsvp = move |status: &'static str| {
        let eid = eid.get_value();
        spawn_local(async move {
            let body = RsvpRequest { status: status.to_string() };
            let _ = post_json::<_, serde_json::Value>(
                &format!("/api/events/{eid}/rsvp"),
                &body,
            ).await;
            refresh.update(|n| *n += 1);
        });
    };

    view! {
        {move || {
            summary.get().flatten().map(|s| {
                let total = s.yes_count + s.maybe_count + s.no_count;
                view! {
                    {(total > 0).then(|| view! {
                        <div class="sched-rsvp-counts">
                            <span class="rsvp-count yes">{s.yes_count}" going"</span>
                            <span class="rsvp-count maybe">{s.maybe_count}" maybe"</span>
                        </div>
                    })}
                }
            })
        }}
        {move || {
            auth.is_member().then(|| {
                view! {
                    <div class="sched-rsvp-actions">
                        <button class="rsvp-btn" on:click=move |_| do_rsvp("yes")>"Going"</button>
                        <button class="rsvp-btn" on:click=move |_| do_rsvp("maybe")>"Maybe"</button>
                        <button class="rsvp-btn" on:click=move |_| do_rsvp("no")>"Can't"</button>
                    </div>
                }
            })
        }}
    }
}
