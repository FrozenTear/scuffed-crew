use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::SummaryCard;
use crate::hooks::{use_api, use_api_list};
use crate::routes::Route;

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

#[derive(Debug, Clone, Deserialize)]
struct RelayHealth {
    configured: bool,
    reachable: bool,
    #[serde(default)]
    forum_backend: String,
}

#[component]
pub fn AdminDashboard() -> Element {
    let members = use_api_list::<Member>("/api/members");
    let applications = use_api_list::<Application>("/api/applications");
    let teams = use_api_list::<Team>("/api/teams");
    let events = use_api_list::<Event>("/api/events");
    let announcements = use_api_list::<Announcement>("/api/announcements");
    let health = use_api::<RelayHealth>("/api/nostr/health");

    let member_count = members
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .map(|v| v.len())
        .unwrap_or(0);
    let pending_count = applications
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .map(|v| v.iter().filter(|a| a.status == "pending").count())
        .unwrap_or(0);
    let team_count = teams
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .map(|v| v.len())
        .unwrap_or(0);
    let event_count = events
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .map(|v| v.len())
        .unwrap_or(0);
    let announcement_count = announcements
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .map(|v| v.len())
        .unwrap_or(0);

    let health_chip = {
        let data = health.data.read();
        let data = data.as_ref().and_then(|d| d.as_ref());
        match data {
            None => None,
            Some(h) => {
                // Local forum + no/offline relay is normal for small installs —
                // soft "optional" chip, not an alarm.
                let forum_local =
                    h.forum_backend.eq_ignore_ascii_case("local") || h.forum_backend.is_empty();
                if !h.configured || (forum_local && !h.reachable) {
                    Some((
                        "dash-chip soft",
                        "Relay optional — forum runs locally. Wire Nostr when ready.",
                    ))
                } else if h.configured && !h.reachable {
                    Some((
                        "dash-chip warn",
                        "Relay configured but offline — check Admin → Relay.",
                    ))
                } else if h.reachable {
                    Some(("dash-chip ok", "Relay online"))
                } else {
                    None
                }
            }
        }
    };

    rsx! {
        h1 { "Dashboard" }

        if let Some((chip_class, chip_text)) = health_chip {
            div { class: "dash-health",
                Link { to: Route::AdminRelay {}, class: "{chip_class}", "{chip_text}" }
            }
        }

        div { class: "summary-cards",
            SummaryCard {
                value: member_count.to_string(),
                label: "Members",
                to: Some(Route::AdminMembers {}),
            }
            SummaryCard {
                value: pending_count.to_string(),
                label: "Pending Apps",
                to: Some(Route::AdminApplications {}),
            }
            SummaryCard {
                value: team_count.to_string(),
                label: "Teams",
                to: Some(Route::AdminTeams {}),
            }
            SummaryCard {
                value: event_count.to_string(),
                label: "Events",
                to: Some(Route::AdminSchedule {}),
            }
            SummaryCard {
                value: announcement_count.to_string(),
                label: "Announcements",
                to: Some(Route::AdminAnnouncements {}),
            }
        }
    }
}
