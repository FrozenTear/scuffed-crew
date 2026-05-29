use leptos::prelude::*;
use serde::Deserialize;

use crate::api;

#[derive(Debug, Clone, Deserialize)]
struct Member {
    #[allow(dead_code)]
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Application {
    status: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Event {
    #[allow(dead_code)]
    title: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Team {
    #[allow(dead_code)]
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Announcement {
    #[allow(dead_code)]
    id: String,
}

#[component]
pub fn DashboardPage() -> impl IntoView {
    let members =
        LocalResource::new(|| async { api::get_list::<Member>("/api/members").await.ok() });
    let apps = LocalResource::new(|| async {
        api::get::<Vec<Application>>("/api/applications").await.ok()
    });
    let events = LocalResource::new(|| async { api::get_list::<Event>("/api/events").await.ok() });
    let teams = LocalResource::new(|| async { api::get_list::<Team>("/api/teams").await.ok() });
    let announcements = LocalResource::new(|| async {
        api::get_list::<Announcement>("/api/announcements")
            .await
            .ok()
    });

    view! {
        <h1>"Dashboard"</h1>
        <div class="summary-cards">
            <div class="summary-card">
                <div class="label">"Members"</div>
                <div class="value">
                    {move || members.get().flatten().map(|m| m.len().to_string()).unwrap_or_else(|| "...".into())}
                </div>
            </div>
            <div class="summary-card">
                <div class="label">"Teams"</div>
                <div class="value">
                    {move || teams.get().flatten().map(|t| t.len().to_string()).unwrap_or_else(|| "...".into())}
                </div>
            </div>
            <div class="summary-card">
                <div class="label">"Pending Apps"</div>
                <div class="value">
                    {move || apps.get().flatten().map(|a| a.iter().filter(|app| app.status == "pending").count().to_string()).unwrap_or_else(|| "...".into())}
                </div>
            </div>
            <div class="summary-card">
                <div class="label">"Events"</div>
                <div class="value">
                    {move || events.get().flatten().map(|e| e.len().to_string()).unwrap_or_else(|| "...".into())}
                </div>
            </div>
            <div class="summary-card">
                <div class="label">"Announcements"</div>
                <div class="value">
                    {move || announcements.get().flatten().map(|a| a.len().to_string()).unwrap_or_else(|| "...".into())}
                </div>
            </div>
        </div>
    }
}
