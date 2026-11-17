use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::ui::{PageHeader, PageShell, PageWidth};
use crate::components::{AdminDate, SummaryCard, admin_pending, load_error_text, loading_text};
use crate::hooks::{ApiResource, use_api, use_api_list};
use crate::routes::Route;
use crate::state::use_auth;
use crate::util::pluralize;

/// The "Needs a look" rows (`d1-admin-dashboard`): the news-list card, with
/// the linked title covering the whole card. The tiles are the shared
/// `.summary-card`.
const DASH_CSS: &str = r#"
.dash-needs {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  min-width: 0;
}
.dash-entries {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  margin: 0;
  padding: 0;
  list-style: none;
  min-width: 0;
}
.dash-entry {
  position: relative;
  box-sizing: border-box;
  min-width: 0;
  padding: var(--space-card-pad);
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
}
.dash-entry h3 {
  margin: 0 0 var(--space-2);
  font-family: var(--font-head);
  font-size: var(--text-lg);
  font-weight: var(--font-label-weight);
  letter-spacing: var(--font-head-tracking);
  line-height: var(--leading-ui);
  color: var(--text);
  overflow-wrap: anywhere;
}
.dash-entry h3 a {
  color: var(--accent-text);
  text-decoration: none;
}
/* The title link covers the whole card, so the target is the card. */
.dash-entry h3 a::after {
  content: "";
  position: absolute;
  inset: 0;
  border-radius: inherit;
}
.dash-entry h3 a:hover { text-decoration: underline; }
.dash-entry:has(h3 a:hover) { border-color: var(--border-strong); }
.dash-entry h3 a:focus-visible { outline: none; }
.dash-entry h3 a:focus-visible::after {
  outline: var(--focus-ring-width) solid var(--focus-ring);
  outline-offset: var(--focus-ring-offset);
}
.dash-entry p,
.dash-note {
  margin: 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-body);
  overflow-wrap: anywhere;
}
/* Nothing to say: out of the layout, still in the tree. */
.dash-note:empty { display: none; }
.dash-note a {
  display: inline-flex;
  align-items: center;
  min-height: var(--hit-target-min);
  color: var(--accent-text);
  text-decoration: underline;
}
"#;

#[derive(Debug, Clone, Deserialize)]
struct Member {
    #[allow(dead_code)]
    id: String,
}

// The list entry also carries the applicant's name and the date; both are
// optional so the page still renders against an older server.
#[derive(Debug, Clone, Deserialize)]
struct Application {
    #[allow(dead_code)]
    id: String,
    status: String,
    #[serde(default)]
    applicant_name: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
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

/// Real counts render as numbers. A count that is still loading or that
/// failed is a labelled tile of its own, never a fake zero. A failed one
/// keeps the server's reason and offers Retry.
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
        KpiDisplay::Loading => {
            let waiting = loading_text(&label.to_lowercase());
            rsx! {
                div {
                    class: "summary-card summary-card-pending is-loading",
                    "data-kpi": "loading",
                    aria_busy: "true",
                    div { class: "summary-card-skel", aria_hidden: "true" }
                    div { class: "label", "{label}" }
                    span { class: "visually-hidden", role: "status", "{waiting}" }
                }
            }
        }
        KpiDisplay::Error => {
            let reason = resource.error.read().clone().unwrap_or_default();
            let message = load_error_text(&label.to_lowercase(), &reason);
            let mut refresh = resource.refresh;
            rsx! {
                div {
                    class: "summary-card summary-card-pending is-error",
                    "data-kpi": "error",
                    role: "alert",
                    div { class: "label", "{label}" }
                    p { class: "summary-card-error", "{message}" }
                    button {
                        r#type: "button",
                        class: "fetch-error__retry is-compact",
                        aria_label: "Retry loading {label}",
                        onclick: move |_| refresh += 1,
                        "Retry"
                    }
                }
            }
        }
    }
}

/// What the relay check says, when it says anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelayNote {
    /// No relay, or a local forum with the relay down: normal for a small
    /// install, not an alarm.
    Optional,
    /// Configured and unreachable. This one needs a look.
    Offline,
    Online,
}

impl RelayNote {
    fn text(self) -> &'static str {
        match self {
            RelayNote::Optional => "Relay optional. Forum runs locally. Wire Nostr when ready.",
            RelayNote::Offline => "The relay is configured but cannot be reached.",
            RelayNote::Online => "Relay online.",
        }
    }
}

/// Optional only while no relay is set up. Once one is configured, DMs and
/// chat depend on it whatever the forum backend is, so a configured relay
/// that cannot be reached needs a look (and agrees with /admin/relay).
fn relay_note(h: &RelayHealth) -> Option<RelayNote> {
    if !h.configured {
        Some(RelayNote::Optional)
    } else if !h.reachable {
        Some(RelayNote::Offline)
    } else {
        Some(RelayNote::Online)
    }
}

enum HealthView {
    Pending,
    /// The health check failed: its reason and Retry show, and "Needs a
    /// look" says it could not check the relay.
    Failed,
    Note(RelayNote),
    Quiet,
}

fn health_view(resource: &ApiResource<RelayHealth>) -> HealthView {
    if resource.error.read().is_some() {
        return HealthView::Failed;
    }
    match resource.data.read().as_ref().and_then(|d| d.as_ref()) {
        None => HealthView::Pending,
        Some(h) => match relay_note(h) {
            Some(note) => HealthView::Note(note),
            None => HealthView::Quiet,
        },
    }
}

/// The pending applications as one "Needs a look" row.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingLook {
    /// "1 application pending".
    title: String,
    /// The newest pending applicant, when the server sent a name.
    newest_name: Option<String>,
    /// When that application was sent.
    newest_at: Option<String>,
    /// More than one is waiting.
    several: bool,
}

/// `None` when nothing is pending.
fn pending_look(applications: &[Application]) -> Option<PendingLook> {
    let pending: Vec<&Application> = applications
        .iter()
        .filter(|a| a.status == "pending")
        .collect();
    if pending.is_empty() {
        return None;
    }
    // ISO timestamps sort as text.
    let newest = pending
        .iter()
        .max_by(|a, b| a.created_at.cmp(&b.created_at))?;
    Some(PendingLook {
        title: format!(
            "{} pending",
            pluralize(pending.len(), "application", "applications")
        ),
        newest_name: newest
            .applicant_name
            .clone()
            .filter(|name| !name.trim().is_empty()),
        newest_at: newest.created_at.clone(),
        several: pending.len() > 1,
    })
}

/// Where one source of "Needs a look" stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Pending,
    Failed,
    Ready,
}

impl Source {
    fn of<T>(error: bool, data: Option<&T>) -> Self {
        if error {
            Source::Failed
        } else if data.is_some() {
            Source::Ready
        } else {
            Source::Pending
        }
    }
}

/// The line under the section title. A source that failed is always said,
/// rows or not: "Nothing needs a look" is never claimed over a check that
/// did not run. Otherwise it is empty when there are rows.
fn needs_status(has_rows: bool, applications: Source, relay: Source) -> String {
    let failed: Vec<&str> = [(applications, "pending applications"), (relay, "the relay")]
        .into_iter()
        .filter(|(source, _)| *source == Source::Failed)
        .map(|(_, what)| what)
        .collect();
    if !failed.is_empty() {
        return format!("Could not check {}.", failed.join(" or "));
    }
    let settled = applications == Source::Ready && relay == Source::Ready;
    match (has_rows, settled) {
        (true, _) => String::new(),
        (false, false) => "Checking what needs a look...".to_string(),
        (false, true) => "Nothing needs a look right now.".to_string(),
    }
}

#[component]
fn PendingEntry(look: PendingLook) -> Element {
    let PendingLook {
        title,
        newest_name,
        newest_at,
        several,
    } = look;
    let tail = if several {
        "Messages are in the applications list."
    } else {
        "Message is in the applications list."
    };
    let detail = match (newest_name, newest_at) {
        (Some(name), Some(at)) => {
            let lead = if several {
                format!("Newest: {name}, ")
            } else {
                format!("{name} applied on ")
            };
            rsx! {
                p {
                    "{lead}"
                    AdminDate { iso: at }
                    ". {tail}"
                }
            }
        }
        (Some(name), None) => {
            let line = if several {
                format!("Newest: {name}. {tail}")
            } else {
                format!("{name} applied. {tail}")
            };
            rsx! { p { "{line}" } }
        }
        _ => rsx! { p { "{tail}" } },
    };
    rsx! {
        li { class: "dash-entry",
            h3 {
                Link { to: Route::AdminApplications {}, "{title}" }
            }
            {detail}
        }
    }
}

/// The quiet relay line. `link` is false for officers: the relay page is
/// admin only, so they get the words without a way in that ends at a wall.
#[component]
fn RelayNoteLine(text: &'static str, link: bool) -> Element {
    rsx! {
        p { class: "dash-note",
            "{text}"
            if link {
                " "
                Link { to: Route::AdminRelay {}, "Open relay status" }
            }
        }
    }
}

/// Title of the "Relay is offline" row; a link for admins only.
#[component]
fn RelayOfflineTitle(link: bool) -> Element {
    rsx! {
        h3 {
            if link {
                Link { to: Route::AdminRelay {}, "Relay is offline" }
            } else {
                "Relay is offline"
            }
        }
    }
}

/// The header lead. Settings is admin only (the page and the sub-nav both
/// gate it on the admin role, QA 706), so officers are not pointed at it.
fn dashboard_lead(is_admin: bool) -> String {
    let rest = if is_admin { "settings" } else { "moderation" };
    format!("A snapshot of the crew. Use the sub-nav for members, applications, and {rest}.")
}

#[component]
pub fn AdminDashboard() -> Element {
    let members = use_api_list::<Member>("/api/members");
    let applications = use_api_list::<Application>("/api/applications");
    let teams = use_api_list::<Team>("/api/teams");
    let events = use_api_list::<Event>("/api/events");
    let announcements = use_api_list::<Announcement>("/api/announcements");
    let health = use_api::<RelayHealth>("/api/nostr/health");

    let relay = health_view(&health);
    let look = applications
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .and_then(|list| pending_look(list));
    let applications_source = Source::of(
        applications.error.read().is_some(),
        applications.data.read().as_ref().and_then(|d| d.as_ref()),
    );
    let relay_source = match relay {
        HealthView::Pending => Source::Pending,
        HealthView::Failed => Source::Failed,
        HealthView::Note(_) | HealthView::Quiet => Source::Ready,
    };
    let relay_offline = matches!(relay, HealthView::Note(RelayNote::Offline));
    let has_rows = look.is_some() || relay_offline;
    let status = needs_status(has_rows, applications_source, relay_source);
    // The relay page is admin only: officers get the words, not a link.
    let relay_link = use_auth()().is_admin();
    let lead = dashboard_lead(relay_link);
    let offline_text = RelayNote::Offline.text();
    // A failed relay check keeps its reason and Retry. A relay that is fine
    // or optional is one quiet line with the way to its page.
    let relay_block = match relay {
        HealthView::Failed => rsx! {
            div { "data-kpi": "health",
                {admin_pending(&health, "relay status")}
            }
        },
        HealthView::Note(note) if note != RelayNote::Offline => {
            rsx! {
                RelayNoteLine { text: note.text(), link: relay_link }
            }
        }
        _ => rsx! {},
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-admin-dashboard", css: DASH_CSS }
        PageShell { width: PageWidth::Wide, class: "admin-dashboard-page",
            PageHeader {
                title: "Admin dashboard",
                lead,
            }

            div { class: "summary-cards",
                {stat_card(&members, "Members", Route::AdminMembers {}, |v| v.len())}
                {stat_card(&applications, "Pending applications", Route::AdminApplications {}, |v| {
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

            section { class: "dash-needs", aria_labelledby: "dash-needs-title",
                h2 { id: "dash-needs-title", class: "ui-section-title", "Needs a look" }
                p { class: "dash-note", role: "status", "{status}" }
                if has_rows {
                    ul { class: "dash-entries",
                        if let Some(look) = look {
                            PendingEntry { look }
                        }
                        if relay_offline {
                            li { class: "dash-entry",
                                RelayOfflineTitle { link: relay_link }
                                p { "{offline_text}" }
                            }
                        }
                    }
                }
                {relay_block}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::admin_shared::test_page::render_admin_route;
    use scuffed_types::OrgRole;

    fn application(status: &str, name: Option<&str>, at: &str) -> Application {
        Application {
            id: format!("app-{at}"),
            status: status.to_string(),
            applicant_name: name.map(str::to_string),
            created_at: Some(at.to_string()),
        }
    }

    #[test]
    fn page_has_one_h1_in_the_wide_shell_and_labelled_loading_tiles() {
        let html = render_admin_route("/admin", Some(OrgRole::Admin));
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide admin-dashboard-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Admin dashboard</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        // Nothing has loaded: five labelled waits, no number yet.
        assert_eq!(html.matches("data-kpi=\"loading\"").count(), 5, "{html}");
        assert!(
            html.contains("role=\"status\">Loading pending applications...</span>"),
            "{html}"
        );
        assert!(!html.contains("class=\"value\""), "{html}");
        // The section says it is still checking, in its status line.
        assert!(
            html.contains(
                "<p class=\"dash-note\" role=\"status\">Checking what needs a look...</p>"
            ),
            "{html}"
        );
    }

    #[test]
    fn needs_a_look_names_the_newest_pending_application() {
        assert_eq!(pending_look(&[]), None);
        assert_eq!(
            pending_look(&[application(
                "accepted",
                Some("Player10"),
                "2026-10-28T10:00:00Z"
            )]),
            None
        );
        let one = pending_look(&[
            application("pending", Some("Player04"), "2026-11-10T10:00:00Z"),
            application("trial", Some("Player09"), "2026-11-12T10:00:00Z"),
        ])
        .expect("one pending");
        assert_eq!(one.title, "1 application pending");
        assert_eq!(one.newest_name.as_deref(), Some("Player04"));
        assert_eq!(one.newest_at.as_deref(), Some("2026-11-10T10:00:00Z"));
        assert!(!one.several);

        let two = pending_look(&[
            application("pending", Some("Player04"), "2026-11-10T10:00:00Z"),
            application("pending", Some("Player05"), "2026-11-11T10:00:00Z"),
        ])
        .expect("two pending");
        assert_eq!(two.title, "2 applications pending");
        assert_eq!(two.newest_name.as_deref(), Some("Player05"));
        assert!(two.several);
    }

    #[test]
    fn needs_status_is_empty_only_when_there_are_rows() {
        use Source::{Pending, Ready};
        assert_eq!(needs_status(true, Pending, Pending), "");
        assert_eq!(needs_status(true, Ready, Pending), "");
        assert_eq!(needs_status(true, Ready, Ready), "");
        assert_eq!(
            needs_status(false, Pending, Pending),
            "Checking what needs a look..."
        );
        assert_eq!(
            needs_status(false, Ready, Pending),
            "Checking what needs a look..."
        );
        assert_eq!(
            needs_status(false, Pending, Ready),
            "Checking what needs a look..."
        );
        assert_eq!(
            needs_status(false, Ready, Ready),
            "Nothing needs a look right now."
        );
    }

    #[test]
    fn a_failed_source_is_said_and_never_reads_as_nothing_to_do() {
        // Vision blocking 12 on PR 706: a failed /api/applications said
        // "Nothing needs a look right now.", and a failed relay check said
        // "Checking..." forever.
        use Source::{Failed, Pending, Ready};
        assert_eq!(
            needs_status(false, Failed, Ready),
            "Could not check pending applications."
        );
        assert_eq!(
            needs_status(false, Ready, Failed),
            "Could not check the relay."
        );
        assert_eq!(
            needs_status(false, Pending, Failed),
            "Could not check the relay."
        );
        assert_eq!(
            needs_status(false, Failed, Failed),
            "Could not check pending applications or the relay."
        );
        // Rows from one source do not hide that the other failed.
        assert_eq!(
            needs_status(true, Ready, Failed),
            "Could not check the relay."
        );
        assert_eq!(Source::of(true, None::<&u8>), Failed);
        assert_eq!(Source::of(true, Some(&1u8)), Failed);
        assert_eq!(Source::of(false, Some(&1u8)), Ready);
        assert_eq!(Source::of(false, None::<&u8>), Pending);
    }

    /// QA's 706 nit: an officer saw "Access Denied" on /admin/settings.
    /// Settings stays admin only, as the sub-nav already had it (the page
    /// denies officers with the shared wall, the sub-nav hides the entry);
    /// the dashboard lead no longer sends officers there.
    #[test]
    fn the_lead_names_settings_only_for_admins() {
        let admin = render_admin_route("/admin", Some(OrgRole::Admin));
        assert!(
            admin.contains("Use the sub-nav for members, applications, and settings."),
            "{admin}"
        );
        let officer = render_admin_route("/admin", Some(OrgRole::Officer));
        assert!(
            officer.contains("Use the sub-nav for members, applications, and moderation."),
            "{officer}"
        );
        assert!(!officer.contains("and settings."), "{officer}");
        assert!(!officer.contains("href=\"/admin/settings\""), "{officer}");
        assert!(officer.contains("href=\"/admin/moderation\""), "{officer}");
        // The page and the sub-nav agree: no link for officers, the wall
        // on a typed URL.
        let denied = render_admin_route("/admin/settings", Some(OrgRole::Officer));
        assert!(
            denied.contains("<h1>You do not have access</h1>"),
            "{denied}"
        );
        assert!(!denied.contains("href=\"/admin/settings\""), "{denied}");
    }

    #[test]
    fn officers_get_relay_words_without_a_link_to_the_admin_only_page() {
        // Vision nit 11 on PR 706.
        use crate::components::ui::test_support::render_routed;
        let admin = render_routed(|| {
            rsx! {
                RelayNoteLine { text: RelayNote::Online.text(), link: true }
                RelayOfflineTitle { link: true }
            }
        });
        assert_eq!(admin.matches("href=\"/admin/relay\"").count(), 2, "{admin}");
        assert!(admin.contains(">Open relay status</a>"), "{admin}");
        let officer = render_routed(|| {
            rsx! {
                RelayNoteLine { text: RelayNote::Online.text(), link: false }
                RelayOfflineTitle { link: false }
            }
        });
        assert!(!officer.contains("/admin/relay"), "{officer}");
        assert!(
            officer.contains("<p class=\"dash-note\">Relay online.</p>"),
            "{officer}"
        );
        assert!(officer.contains("<h3>Relay is offline</h3>"), "{officer}");
    }

    #[test]
    fn only_a_configured_relay_that_is_down_needs_a_look() {
        let health = |configured, reachable, backend: &str| RelayHealth {
            configured,
            reachable,
            forum_backend: backend.to_string(),
        };
        assert_eq!(
            relay_note(&health(false, false, "local")),
            Some(RelayNote::Optional)
        );
        // Configured but down with a local forum: DMs and chat still need it.
        assert_eq!(
            relay_note(&health(true, false, "local")),
            Some(RelayNote::Offline)
        );
        assert_eq!(
            relay_note(&health(true, true, "local")),
            Some(RelayNote::Online)
        );
        assert_eq!(
            relay_note(&health(true, false, "nostr")),
            Some(RelayNote::Offline)
        );
        assert_eq!(
            relay_note(&health(true, true, "nostr")),
            Some(RelayNote::Online)
        );
    }

    #[test]
    fn dashboard_css_is_on_tokens() {
        crate::components::ui::test_support::assert_sizes_from_tokens(DASH_CSS);
    }

    #[test]
    fn fetch_error_is_not_zero_and_zero_is_not_loading() {
        assert_eq!(kpi_display(false, None), KpiDisplay::Loading);
        assert_eq!(kpi_display(true, None), KpiDisplay::Error);
        assert_eq!(kpi_display(true, Some(0)), KpiDisplay::Error);
        assert_eq!(kpi_display(false, Some(0)), KpiDisplay::Value(0));
        assert_eq!(kpi_display(false, Some(4)), KpiDisplay::Value(4));
    }
}
