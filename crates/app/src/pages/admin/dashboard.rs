use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::{SummaryCard, ADMIN_SHARED_CSS};
use crate::hooks::use_api;

#[derive(Debug, Clone, Deserialize)]
struct Member {
    #[allow(dead_code)]
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Application {
    #[allow(dead_code)]
    id: String,
    status: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Team {
    #[allow(dead_code)]
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Event {
    #[allow(dead_code)]
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Announcement {
    #[allow(dead_code)]
    id: String,
}

#[component]
pub fn AdminDashboard() -> Element {
    let members = use_api::<Vec<Member>>("/api/members");
    let applications = use_api::<Vec<Application>>("/api/applications");
    let teams = use_api::<Vec<Team>>("/api/teams");
    let events = use_api::<Vec<Event>>("/api/events");
    let announcements = use_api::<Vec<Announcement>>("/api/announcements");

    let member_count = members.data.read().as_ref()
        .and_then(|d| d.as_ref()).map(|v| v.len()).unwrap_or(0);
    let pending_count = applications.data.read().as_ref()
        .and_then(|d| d.as_ref())
        .map(|v| v.iter().filter(|a| a.status == "pending").count())
        .unwrap_or(0);
    let team_count = teams.data.read().as_ref()
        .and_then(|d| d.as_ref()).map(|v| v.len()).unwrap_or(0);
    let event_count = events.data.read().as_ref()
        .and_then(|d| d.as_ref()).map(|v| v.len()).unwrap_or(0);
    let announcement_count = announcements.data.read().as_ref()
        .and_then(|d| d.as_ref()).map(|v| v.len()).unwrap_or(0);

    rsx! {
        style { {ADMIN_SHARED_CSS} }

        h1 { "Dashboard" }
        div { class: "summary-cards",
            SummaryCard { value: member_count.to_string(), label: "Members" }
            SummaryCard { value: pending_count.to_string(), label: "Pending Apps" }
            SummaryCard { value: team_count.to_string(), label: "Teams" }
            SummaryCard { value: event_count.to_string(), label: "Events" }
            SummaryCard { value: announcement_count.to_string(), label: "Announcements" }
        }
    }
}
