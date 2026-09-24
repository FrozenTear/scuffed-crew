use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::{SummaryCard, admin_pending};
use crate::hooks::{ApiResource, use_api, use_api_list};
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

/// Loading, failed fetch, and a real count (including zero) are different.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KpiDisplay {
    Loading,
    Error,
    Value(usize),
}

fn kpi_display(has_error: bool, value: Option<usize>) -> KpiDisplay {
    if has_error {
        KpiDisplay::Error
    } else if let Some(value) = value {
        KpiDisplay::Value(value)
    } else {
        KpiDisplay::Loading
    }
}

fn list_kpi<T>(resource: &ApiResource<Vec<T>>, count: impl FnOnce(&[T]) -> usize) -> KpiDisplay {
    let has_error = resource.error.read().is_some();
    let value = if has_error {
        None
    } else {
        resource
            .data
            .read()
            .as_ref()
            .and_then(|d| d.as_ref())
            .map(|items| count(items))
    };
    kpi_display(has_error, value)
}

fn pending_card_class(kind: KpiDisplay) -> &'static str {
    match kind {
        KpiDisplay::Error => "summary-card summary-card-pending is-error",
        KpiDisplay::Loading | KpiDisplay::Value(_) => {
            "summary-card summary-card-pending is-loading"
        }
    }
}

/// Real counts render as numbers. `None` data is loading or error via
/// `admin_pending` — never a fake zero.
fn stat_card<T: 'static>(
    resource: &ApiResource<Vec<T>>,
    label: &'static str,
    to: Route,
    count: impl FnOnce(&[T]) -> usize,
) -> Element {
    match list_kpi(resource, count) {
        KpiDisplay::Value(n) => rsx! {
            SummaryCard { value: n.to_string(), label, to: Some(to) }
        },
        kind => {
            let class = pending_card_class(kind);
            let busy = matches!(kind, KpiDisplay::Loading);
            rsx! {
                div {
                    class: "{class}",
                    "data-kpi": if busy { "loading" } else { "error" },
                    aria_busy: if busy { "true" } else { "false" },
                    aria_live: "polite",
                    div { class: "label", "{label}" }
                    {admin_pending(resource, label)}
                }
            }
        }
    }
}

enum HealthView {
    Pending,
    Chip(&'static str, &'static str),
    Quiet,
}

fn relay_chip(h: &RelayHealth) -> Option<(&'static str, &'static str)> {
    // Local forum + no/offline relay is normal for small installs —
    // soft "optional" chip, not an alarm.
    let forum_local = h.forum_backend.eq_ignore_ascii_case("local") || h.forum_backend.is_empty();
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

fn health_view(resource: &ApiResource<RelayHealth>) -> HealthView {
    if resource.error.read().is_some() {
        return HealthView::Pending;
    }
    match resource.data.read().as_ref().and_then(|d| d.as_ref()) {
        None => HealthView::Pending,
        Some(h) => match relay_chip(h) {
            Some((class, text)) => HealthView::Chip(class, text),
            None => HealthView::Quiet,
        },
    }
}

#[component]
pub fn AdminDashboard() -> Element {
    let members = use_api_list::<Member>("/api/members");
    let applications = use_api_list::<Application>("/api/applications");
    let teams = use_api_list::<Team>("/api/teams");
    let events = use_api_list::<Event>("/api/events");
    let announcements = use_api_list::<Announcement>("/api/announcements");
    let health = use_api::<RelayHealth>("/api/nostr/health");

    rsx! {
        h1 { "Dashboard" }

        match health_view(&health) {
            HealthView::Pending => rsx! {
                div { class: "dash-health", "data-kpi": "health",
                    {admin_pending(&health, "relay status")}
                }
            },
            HealthView::Chip(chip_class, chip_text) => rsx! {
                div { class: "dash-health",
                    Link { to: Route::AdminRelay {}, class: "{chip_class}", "{chip_text}" }
                }
            },
            HealthView::Quiet => rsx! {},
        }

        div { class: "summary-cards",
            {stat_card(&members, "Members", Route::AdminMembers {}, |v| v.len())}
            {stat_card(&applications, "Pending Apps", Route::AdminApplications {}, |v| {
                v.iter().filter(|a| a.status == "pending").count()
            })}
            {stat_card(&teams, "Teams", Route::AdminTeams {}, |v| v.len())}
            {stat_card(&events, "Events", Route::AdminSchedule {}, |v| v.len())}
            {stat_card(
                &announcements,
                "Announcements",
                Route::AdminAnnouncements {},
                |v| v.len(),
            )}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{KpiDisplay, kpi_display};

    #[test]
    fn fetch_error_is_not_zero_and_zero_is_not_loading() {
        assert_eq!(kpi_display(false, None), KpiDisplay::Loading);
        assert_eq!(kpi_display(true, None), KpiDisplay::Error);
        assert_eq!(kpi_display(true, Some(0)), KpiDisplay::Error);
        assert_eq!(kpi_display(false, Some(0)), KpiDisplay::Value(0));
        assert_eq!(kpi_display(false, Some(4)), KpiDisplay::Value(4));
    }
}
