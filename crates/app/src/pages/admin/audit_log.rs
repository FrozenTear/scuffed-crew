//! Admin: the audit log (`/admin/audit-log`, admin only).
//!
//! The server pages the log (`limit` and `offset`, with the full `total`), so
//! the pager walks the whole log. The kind chips and the search box have no
//! server side yet: they narrow the page that is loaded, and an always
//! rendered status line says how many of its entries match.

use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::ui::{
    ChipButton, ChipRow, EmptyCard, PageHeader, PageShell, PageWidth, Pager, btn_class,
    show_list_top,
};
use crate::components::{AccessDenied, DataTable, admin_list_pending, admin_short_date};
use crate::hooks::{use_api_list, use_api_with};
use crate::routes::Route;
use crate::state::use_auth;

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct AuditLogEntry {
    id: String,
    #[allow(dead_code)]
    actor_id: String,
    actor_name: String,
    action: String,
    target_type: String,
    target_id: String,
    /// Resolved target display name (server enriches on read; empty/id = no label).
    #[serde(default)]
    target_label: String,
    details: Option<String>,
    created_at: String,
}

/// A member as `/api/members` returns them, for names in the details.
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct NamedMember {
    id: String,
    display_name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct AuditLogResponse {
    entries: Vec<AuditLogEntry>,
    total: u64,
}

/// Entries per server page. The filters only see the loaded page, so this
/// stays at the old 50 rather than the 20 of other admin lists.
const PAGE_SIZE: usize = 50;

/// The list region: the pager's focus target after a page change.
const LIST_ID: &str = "admin-audit-list";
const SEARCH_ID: &str = "audit-search";
const FILTER_STATUS_ID: &str = "audit-filter-status";

const CSS: &str = r#"
    .admin-audit-page .audit-filter-status {
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
    }
    /* No text: out of the layout, still in the tree (a live line
       that is display: none drops out of what screen readers track). */
    .admin-audit-page .audit-filter-status:empty {
        position: absolute;
        width: 1px;
        height: 1px;
        padding: 0;
        margin: -1px;
        overflow: hidden;
        clip: rect(0, 0, 0, 0);
        white-space: nowrap;
        border: 0;
    }
    /* The record id of an unnamed target: a muted line under the words. */
    .admin-audit-page .audit-target-id {
        display: block;
        color: var(--text-3);
        font-size: var(--text-sm);
        overflow-wrap: anywhere;
    }
    .admin-audit-page .admin-toolbar-search {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
    }
    /* The search label sits above its field: line the chips up with the
       field, not with the label. */
    .admin-audit-page .admin-toolbar { align-items: flex-end; }
"#;

/// What kind of change an action is. The chips filter on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionKind {
    All,
    Created,
    Changed,
    Deleted,
    Viewed,
}

impl ActionKind {
    fn value(self) -> &'static str {
        match self {
            ActionKind::All => "all",
            ActionKind::Created => "created",
            ActionKind::Changed => "changed",
            ActionKind::Deleted => "deleted",
            ActionKind::Viewed => "viewed",
        }
    }

    fn word(self) -> &'static str {
        match self {
            ActionKind::All => "All",
            ActionKind::Created => "Created",
            ActionKind::Changed => "Changed",
            ActionKind::Deleted => "Deleted",
            ActionKind::Viewed => "Viewed",
        }
    }
}

const KINDS: [ActionKind; 5] = [
    ActionKind::All,
    ActionKind::Created,
    ActionKind::Changed,
    ActionKind::Deleted,
    ActionKind::Viewed,
];

/// The kind of a stored action (`created_game`, `viewed_member_matches`),
/// from its first word. Anything that is not an add, a removal or a read is
/// a change.
fn action_kind(action: &str) -> ActionKind {
    let verb = action.split('_').next().unwrap_or_default();
    match verb {
        "created" | "added" | "recorded" | "generated" | "uploaded" | "sent" => ActionKind::Created,
        "deleted" | "removed" | "revoked" => ActionKind::Deleted,
        "viewed" | "read" | "downloaded" => ActionKind::Viewed,
        _ => ActionKind::Changed,
    }
}

/// `created_game` reads "Created game".
fn humanize(raw: &str) -> String {
    let words = raw.trim().replace('_', " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// "Team: Squad Alpha". With no label from the server (label empty or equal
/// to the id) it says so, and never prints the record id.
fn target_text(entry: &AuditLogEntry) -> String {
    let kind = humanize(&entry.target_type);
    if !entry.target_label.trim().is_empty() && entry.target_label != entry.target_id {
        format!("{kind}: {}", entry.target_label)
    } else {
        format!("{kind} (no name recorded)")
    }
}

/// The record id of a target that has no name recorded, shown as a muted
/// second line so two unnamed targets can still be told apart. `None` when
/// the target has a name (the name is enough) or no id.
fn target_id_line(entry: &AuditLogEntry) -> Option<String> {
    let named = !entry.target_label.trim().is_empty() && entry.target_label != entry.target_id;
    let id = entry.target_id.trim();
    (!named && !id.is_empty()).then(|| format!("ID {id}"))
}

/// `member:abc` and `abc` are the same record.
fn bare_member_id(id: &str) -> &str {
    id.trim().strip_prefix("member:").unwrap_or(id.trim())
}

/// The details line in words: member ids become their names, and a line
/// that is only request parameters (`offset=0 limit=25 season=all`) reads
/// "offset 0, limit 25, season all".
fn details_text(details: &str, members: &[NamedMember]) -> String {
    let words: Vec<&str> = details.split_whitespace().collect();
    if !words.is_empty()
        && words
            .iter()
            .all(|w| w.split_once('=').is_some_and(|(k, _)| !k.is_empty()))
    {
        return words
            .iter()
            .filter_map(|w| w.split_once('='))
            .map(|(k, v)| format!("{} {}", k.replace('_', " "), v))
            .collect::<Vec<_>>()
            .join(", ");
    }
    words
        .iter()
        .map(|w| {
            let core = w.trim_end_matches(['.', ',', ';', ':']);
            let tail = &w[core.len()..];
            members
                .iter()
                .find(|m| !core.is_empty() && bare_member_id(&m.id) == bare_member_id(core))
                .map(|m| format!("{}{tail}", m.display_name))
                .unwrap_or_else(|| (*w).to_string())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `2026-11-10T19:05:00Z` reads "10 Nov 2026 19:05" (UTC, as stored). A
/// value that is not a timestamp is shown as it is.
fn time_text(iso: &str) -> String {
    let Some((_, day)) = admin_short_date(iso) else {
        return iso.to_string();
    };
    match iso.get(11..16) {
        Some(clock) if clock.as_bytes().get(2) == Some(&b':') => format!("{day} {clock}"),
        _ => day,
    }
}

/// True when the entry is of this kind and the search text (any case)
/// appears in its actor, action, target or details.
fn entry_matches(entry: &AuditLogEntry, kind: ActionKind, query: &str) -> bool {
    if kind != ActionKind::All && action_kind(&entry.action) != kind {
        return false;
    }
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    [
        entry.actor_name.clone(),
        entry.action.clone(),
        humanize(&entry.action),
        target_text(entry),
        entry.details.clone().unwrap_or_default(),
    ]
    .iter()
    .any(|field| field.to_lowercase().contains(&query))
}

/// The filter status line. Empty while no filter is on.
fn filter_status_text(filtering: bool, matched: usize, loaded: usize) -> String {
    if !filtering {
        return String::new();
    }
    let noun = if loaded == 1 { "entry" } else { "entries" };
    format!("{matched} of {loaded} {noun} on this page match the filters.")
}

/// Admin only. The role is checked before any request: an officer gets the
/// no-access page without first asking the API for the log (a 403).
#[component]
pub fn AdminAuditLog() -> Element {
    let auth = use_auth();
    if !auth().is_admin() {
        return rsx! {
            AccessDenied { message: "You need admin permissions to view the audit log.".to_string() }
        };
    }
    rsx! { AuditLogPage {} }
}

#[component]
fn AuditLogPage() -> Element {
    let mut page = use_signal(|| 1usize);
    let mut kind = use_signal(|| ActionKind::All);
    let mut query = use_signal(String::new);

    let log_data = use_api_with::<AuditLogResponse>(move || {
        let offset = page().saturating_sub(1) * PAGE_SIZE;
        format!("/api/audit-log?limit={PAGE_SIZE}&offset={offset}")
    });
    // Names for member ids in the details, inactive members included.
    let members = use_api_list::<NamedMember>("/api/members?include_inactive=true");
    let member_names: Vec<NamedMember> = members
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .cloned()
        .unwrap_or_default();

    let filtering = kind() != ActionKind::All || !query().trim().is_empty();
    let mut clear_filters = move || {
        kind.set(ActionKind::All);
        query.set(String::new());
    };

    let (list, status) = {
        let data = log_data.data.read();
        match data.as_ref().and_then(|d| d.as_ref()) {
            None => (
                admin_list_pending(&log_data, "the audit log"),
                String::new(),
            ),
            Some(resp) => {
                let total = usize::try_from(resp.total).unwrap_or(usize::MAX);
                let rows: Vec<AuditLogEntry> = resp
                    .entries
                    .iter()
                    .filter(|e| entry_matches(e, kind(), &query()))
                    .cloned()
                    .collect();
                let status = filter_status_text(filtering, rows.len(), resp.entries.len());
                let body = if total == 0 && resp.entries.is_empty() {
                    rsx! {
                        EmptyCard {
                            title: "No audit entries yet",
                            message: "Entries appear here when an officer or admin changes members, teams, events, applications or settings. Make a change from the dashboard and it is logged here.",
                            actions: rsx! {
                                Link { to: Route::AdminDashboard {}, class: btn_class(false), "Open the dashboard" }
                            },
                        }
                    }
                } else if resp.entries.is_empty() {
                    rsx! {
                        EmptyCard {
                            title: "This page has no entries",
                            message: "The log is shorter than this page. Go back to the newest entries.",
                            actions: rsx! {
                                button {
                                    r#type: "button",
                                    class: "btn is-ghost",
                                    onclick: move |_| {
                                        page.set(1);
                                        show_list_top(LIST_ID);
                                    },
                                    "Go to the first page"
                                }
                            },
                        }
                    }
                } else if rows.is_empty() {
                    rsx! {
                        EmptyCard {
                            title: "No entries on this page match these filters",
                            message: "Clear the filters to see every entry on this page, or move to another page.",
                            actions: rsx! {
                                button {
                                    r#type: "button",
                                    class: "btn is-ghost",
                                    onclick: move |_| clear_filters(),
                                    "Clear filters"
                                }
                            },
                        }
                    }
                } else {
                    rsx! { AuditTable { rows, members: member_names.clone() } }
                };
                (
                    rsx! {
                        {body}
                        Pager {
                            id: "audit-pager",
                            page: page(),
                            total,
                            page_size: PAGE_SIZE,
                            noun: "entries",
                            noun_singular: "entry",
                            show_page_count: true,
                            on_change: move |next| {
                                page.set(next);
                                show_list_top(LIST_ID);
                            },
                        }
                    },
                    status,
                )
            }
        }
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-admin-audit_log-css", css: CSS }
        PageShell { width: PageWidth::Wide, class: "admin-audit-page",
            PageHeader {
                title: "Audit log",
                lead: "Every change an officer or admin made, and the admin views that are logged, newest first. Times are UTC. The filters narrow the page that is loaded.",
            }
            div { class: "admin-toolbar",
                ChipRow { label: "Action filters", scroll: true,
                    for k in KINDS {
                        ChipButton {
                            key: "{k.value()}",
                            class: "chip",
                            id: format!("audit-filter-{}", k.value()),
                            label: k.word(),
                            pressed: kind() == k,
                            onclick: move |_| kind.set(k),
                        }
                    }
                }
                div { class: "admin-toolbar-search",
                    label { class: "field-label", r#for: SEARCH_ID, "Search this page" }
                    input {
                        class: "field-input",
                        r#type: "search",
                        id: SEARCH_ID,
                        aria_describedby: FILTER_STATUS_ID,
                        value: "{query}",
                        oninput: move |evt| query.set(evt.value()),
                    }
                }
            }
            p { id: FILTER_STATUS_ID, class: "audit-filter-status", role: "status", "{status}" }
            section {
                id: LIST_ID,
                class: "admin-list pager-target",
                tabindex: "-1",
                aria_label: "Audit log entries",
                {list}
            }
        }
    }
}

/// The entries: actor, action, target, details and time. On phone each
/// entry is a card: the actor on top, then "Created game · 10 Nov 2026
/// 19:05", then the target and details lines.
#[component]
fn AuditTable(rows: Vec<AuditLogEntry>, #[props(default)] members: Vec<NamedMember>) -> Element {
    rsx! {
        DataTable {
            headers: vec!["Actor", "Action", "Target", "Details", "Time (UTC)"],
            num_cols: vec![4],
            cards: true,
            label: "Audit log",
            for entry in rows.iter() {
                tr { key: "{entry.id}",
                    td { class: "cell-who", "{entry.actor_name}" }
                    td { class: "cell-meta", {humanize(&entry.action)} }
                    td { "data-label": "Target",
                        {target_text(entry)}
                        if let Some(id) = target_id_line(entry) {
                            span { class: "audit-target-id", "{id}" }
                        }
                    }
                    td { "data-label": "Details",
                        {details_text(entry.details.as_deref().unwrap_or_default(), &members)}
                    }
                    td { class: "cell-meta num",
                        time { class: "date", datetime: "{entry.created_at}", {time_text(&entry.created_at)} }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{assert_sizes_from_tokens, render_routed};

    fn entry(id: &str, action: &str, details: Option<&str>) -> AuditLogEntry {
        AuditLogEntry {
            id: id.into(),
            actor_id: "member:p1".into(),
            actor_name: "Player01".into(),
            action: action.into(),
            target_type: "team".into(),
            target_id: "team:t1".into(),
            target_label: "Squad Alpha".into(),
            details: details.map(Into::into),
            created_at: "2026-11-10T19:05:00Z".into(),
        }
    }

    #[test]
    fn empty_status_line_stays_in_the_tree() {
        // Vision nit 5 on PR 706: `display: none` dropped the live line.
        crate::styles::admin::assert_live_line_kept(
            super::CSS,
            ".admin-audit-page .audit-filter-status:empty {",
        );
    }

    #[test]
    fn page_has_one_h1_filters_a_status_line_and_a_labelled_wait() {
        use crate::components::admin_shared::test_page::render_admin_route;
        let html = render_admin_route("/admin/audit-log", Some(scuffed_types::OrgRole::Admin));
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide admin-audit-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Audit log</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(
            html.contains("role=\"group\" aria-label=\"Action filters\""),
            "{html}"
        );
        assert!(
            html.contains("id=\"audit-filter-all\" aria-pressed=\"true\""),
            "{html}"
        );
        // The search field has a visible label tied by id.
        assert!(html.contains("for=\"audit-search\""), "{html}");
        assert!(html.contains("id=\"audit-search\""), "{html}");
        // The filter status line is in the tree before it has anything to say.
        assert!(
            html.contains(
                "<p id=\"audit-filter-status\" class=\"audit-filter-status\" role=\"status\"></p>"
            ),
            "{html}"
        );
        assert!(
            html.contains("role=\"status\">Loading the audit log...</p>"),
            "{html}"
        );
    }

    #[test]
    fn officers_get_the_no_access_page() {
        use crate::components::admin_shared::test_page::render_admin_route;
        let html = render_admin_route("/admin/audit-log", Some(scuffed_types::OrgRole::Officer));
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("You need admin permissions"), "{html}");
        assert!(!html.contains("audit-search"), "{html}");
    }

    #[test]
    fn actions_sort_into_kinds_and_read_as_words() {
        assert_eq!(action_kind("created_game"), ActionKind::Created);
        assert_eq!(action_kind("added_to_roster"), ActionKind::Created);
        assert_eq!(action_kind("deleted_all_hero_icons"), ActionKind::Deleted);
        assert_eq!(action_kind("revoked_daemon_token"), ActionKind::Deleted);
        assert_eq!(action_kind("viewed_member_matches"), ActionKind::Viewed);
        assert_eq!(action_kind("downloaded_stat_report"), ActionKind::Viewed);
        assert_eq!(action_kind("updated_settings"), ActionKind::Changed);
        assert_eq!(action_kind("nostr_key_linked"), ActionKind::Changed);
        assert_eq!(humanize("created_game"), "Created game");
        assert_eq!(humanize("forum_thread"), "Forum thread");
    }

    #[test]
    fn filters_match_kind_and_any_text_field() {
        let made = entry("a1", "created_team", Some("Season 3 roster"));
        let gone = entry("a2", "deleted_event", None);
        assert!(entry_matches(&made, ActionKind::All, ""));
        assert!(entry_matches(&made, ActionKind::Created, ""));
        assert!(!entry_matches(&gone, ActionKind::Created, ""));
        assert!(entry_matches(&made, ActionKind::All, "season 3"));
        assert!(entry_matches(&made, ActionKind::All, "squad"));
        assert!(entry_matches(&made, ActionKind::All, "player01"));
        assert!(entry_matches(&made, ActionKind::All, "created team"));
        assert!(!entry_matches(&gone, ActionKind::All, "season"));
        assert_eq!(filter_status_text(false, 1, 50), "");
        assert_eq!(
            filter_status_text(true, 3, 50),
            "3 of 50 entries on this page match the filters."
        );
    }

    #[test]
    fn target_and_time_read_plainly() {
        let mut e = entry("a1", "updated_team", None);
        assert_eq!(target_text(&e), "Team: Squad Alpha");
        e.target_label = "team:t1".into();
        assert_eq!(target_text(&e), "Team (no name recorded)");
        e.target_label = String::new();
        assert!(!target_text(&e).contains("t1"), "never the record id");
        // The id is its own muted line instead (Vision blocking 8 on PR 706:
        // two unnamed targets could not be told apart).
        assert_eq!(target_id_line(&e).as_deref(), Some("ID team:t1"));
        e.target_label = "Squad Alpha".into();
        assert_eq!(target_id_line(&e), None);
        e.target_label = String::new();
        e.target_id = " ".into();
        assert_eq!(target_id_line(&e), None);
        assert_eq!(time_text("2026-11-10T19:05:00Z"), "10 Nov 2026 19:05");
        assert_eq!(time_text("2026-11-10"), "10 Nov 2026");
        assert_eq!(time_text("soon"), "soon");
    }

    #[test]
    fn details_name_members_and_read_parameters_as_words() {
        let members = vec![NamedMember {
            id: "member:mb0g1akg5eglvuix88qy".into(),
            display_name: "Player07".into(),
        }];
        assert_eq!(
            details_text("Lifted ban on member mb0g1akg5eglvuix88qy", &members),
            "Lifted ban on member Player07"
        );
        assert_eq!(
            details_text("offset=0 limit=25 season=all", &members),
            "offset 0, limit 25, season all"
        );
        assert_eq!(details_text("", &members), "");
        assert_eq!(details_text("Changed role.", &[]), "Changed role.");
    }

    #[test]
    fn rows_are_cards_with_the_actor_on_top_and_a_right_aligned_time() {
        let html = render_routed(|| {
            rsx! {
                AuditTable {
                    rows: vec![
                        entry("a1", "created_team", Some("Season 3 roster")),
                        entry("a2", "deleted_event", None),
                    ],
                }
            }
        });
        assert!(html.contains("data-table-scroll is-cards"), "{html}");
        assert!(html.contains("aria-label=\"Audit log\""), "{html}");
        assert!(
            html.contains("<th scope=\"col\" class=\"num\">Time (UTC)</th>"),
            "{html}"
        );
        assert!(
            html.contains("<td class=\"cell-who\">Player01</td>"),
            "{html}"
        );
        assert!(
            html.contains("<td class=\"cell-meta\">Created team</td>"),
            "{html}"
        );
        assert!(
            html.contains("<td data-label=\"Target\">Team: Squad Alpha</td>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<time class=\"date\" datetime=\"2026-11-10T19:05:00Z\">10 Nov 2026 19:05</time>"
            ),
            "{html}"
        );
        // No details: the cell is empty, so the phone card drops the line.
        assert!(html.contains("<td data-label=\"Details\"></td>"), "{html}");
    }

    #[test]
    fn page_css_sizes_come_from_tokens() {
        assert_sizes_from_tokens(CSS);
    }
}
