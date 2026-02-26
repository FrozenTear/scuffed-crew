use leptos::prelude::*;
use serde::Deserialize;

use crate::api;
use crate::components::data_table::DataTable;

#[derive(Debug, Clone, Deserialize)]
struct Event {
    id: String,
    title: String,
    day_of_week: u8,
    time: String,
    timezone: String,
    is_recurring: bool,
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
    let events = LocalResource::new(|| async { api::get::<Vec<Event>>("/api/events").await.ok() });

    view! {
        <h1>"Schedule"</h1>
        {move || match events.get().flatten() {
            None => view! { <p>"Loading..."</p> }.into_any(),
            Some(list) if list.is_empty() => view! { <p>"No events scheduled."</p> }.into_any(),
            Some(list) => view! {
                <DataTable headers=vec!["Title", "Day", "Time", "Recurring"]>
                    {list.into_iter().map(|e| {
                        let recurring = if e.is_recurring { "Yes" } else { "No" };
                        let time_display = format!("{} {}", e.time, e.timezone);
                        view! {
                            <tr>
                                <td>{e.title}</td>
                                <td>{day_name(e.day_of_week)}</td>
                                <td>{time_display}</td>
                                <td>{recurring}</td>
                            </tr>
                        }
                    }).collect_view()}
                </DataTable>
            }.into_any(),
        }}
    }
}
