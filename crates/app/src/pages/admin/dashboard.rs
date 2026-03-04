use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;
use crate::components::{SummaryCard, ADMIN_SHARED_CSS};

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
    let members = use_resource(|| async {
        ApiClient::web().fetch::<Vec<Member>>("/api/members").await.ok()
    });
    let applications = use_resource(|| async {
        ApiClient::web().fetch::<Vec<Application>>("/api/applications").await.ok()
    });
    let teams = use_resource(|| async {
        ApiClient::web().fetch::<Vec<Team>>("/api/teams").await.ok()
    });
    let events = use_resource(|| async {
        ApiClient::web().fetch::<Vec<Event>>("/api/events").await.ok()
    });
    let announcements = use_resource(|| async {
        ApiClient::web().fetch::<Vec<Announcement>>("/api/announcements").await.ok()
    });

    let member_count = members.read().as_ref()
        .and_then(|d| d.as_ref()).map(|v| v.len()).unwrap_or(0);
    let pending_count = applications.read().as_ref()
        .and_then(|d| d.as_ref())
        .map(|v| v.iter().filter(|a| a.status == "pending").count())
        .unwrap_or(0);
    let team_count = teams.read().as_ref()
        .and_then(|d| d.as_ref()).map(|v| v.len()).unwrap_or(0);
    let event_count = events.read().as_ref()
        .and_then(|d| d.as_ref()).map(|v| v.len()).unwrap_or(0);
    let announcement_count = announcements.read().as_ref()
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
