use leptos::prelude::*;
use serde::Deserialize;

use scuffed_auth::client::api::fetch_json;

use crate::components::SectionHeader;

#[derive(Debug, Clone, Deserialize)]
struct Event {
    title: String,
    day_of_week: u8,
    time: String,
    timezone: String,
}

const DAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

#[component]
pub fn Schedule() -> impl IntoView {
    let events = LocalResource::new(|| async {
        fetch_json::<Vec<Event>>("/api/events").await.ok()
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
                        let day_events: Vec<&Event> = event_list
                            .iter()
                            .filter(|e| e.day_of_week == day)
                            .collect();

                        let day_name = DAY_NAMES[day as usize];

                        if let Some(event) = day_events.first() {
                            let time_display = format!("{} {}", event.time, event.timezone);
                            view! {
                                <div class="sched-active">
                                    <div class="day-label">{day_name}</div>
                                    <div class="day-event">{event.title.clone()}</div>
                                    <div class="day-time">{time_display}</div>
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
        </section>
    }
}
