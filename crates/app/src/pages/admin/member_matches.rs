//! Officer view of one member's single matches.
//!
//! Reads `GET /api/stats/member/{id}/matches` (officer only). A member whose
//! `stats_visibility` is `private` answers 403 `hidden_by_member`. The page then
//! says so calmly and still shows the totals from `GET /api/stats/member/{id}`,
//! which officers may read for every visibility. An officer's own id always loads.

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::ui::{
    EmptyCard, PageHeader, PageShell, PageWidth, Pill, PillTone, SkeletonBars, show_list_top,
};
use crate::components::{DataTable, SummaryCard, is_http_status};
use crate::hooks::use_api_with;
use crate::routes::Route;
use crate::util::encode_query;
use scuffed_types::STATS_HIDDEN_BY_MEMBER;

/// Rows per page. The server accepts 1 to 100.
const PAGE_LIMIT: u32 = 25;

/// Calm copy for a member who hides single matches. Totals stay visible.
pub(crate) const HIDDEN_COPY: &str =
    "This member keeps their single matches private. Their totals are still shown below.";

#[derive(Debug, Clone, Deserialize)]
struct MemberName {
    /// The record id from the response. Element ids are built from this, never
    /// from the raw URL segment.
    id: String,
    display_name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Totals {
    total_matches: u32,
    wins: u32,
    losses: u32,
    draws: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct MemberMatch {
    id: String,
    hero: String,
    map_name: String,
    #[serde(default)]
    role: String,
    outcome: String,
    elims: u32,
    deaths: u32,
    assists: u32,
    damage: u32,
    healing: u32,
    played_at: DateTime<Utc>,
    /// Cell names the reader flagged. Missing or null is empty.
    #[serde(default)]
    suspect_fields: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
struct MatchPage {
    data: Vec<MemberMatch>,
    #[serde(default)]
    next_cursor: Option<String>,
}

/// `GET` path for one page of a member's matches.
fn matches_path(member_id: &str, cursor: Option<&str>) -> String {
    let base = format!(
        "/api/stats/member/{}/matches?limit={PAGE_LIMIT}",
        encode_query(member_id)
    );
    match cursor {
        Some(c) => format!("{base}&cursor={}", encode_query(c)),
        None => base,
    }
}

fn totals_path(member_id: &str) -> String {
    format!("/api/stats/member/{}", encode_query(member_id))
}

fn name_path(member_id: &str) -> String {
    format!("/api/members/{}", encode_query(member_id))
}

/// Calm copy for a member id that does not exist.
const NOT_FOUND_COPY: &str = "This member was not found.";

/// True when a `use_api*` error string is the 403 for a member who hides matches.
fn is_hidden_by_member(error: &str) -> bool {
    is_http_status(error, 403) && error.contains(STATS_HIDDEN_BY_MEMBER)
}

/// True when the member does not exist: either request answered 404.
fn is_not_found(matches_error: Option<&str>, name_error: Option<&str>) -> bool {
    [matches_error, name_error]
        .into_iter()
        .flatten()
        .any(|err| is_http_status(err, 404))
}

/// A retry only helps a failure that can pass: not a hidden member and not a
/// member who does not exist.
fn can_retry(error: Option<&str>) -> bool {
    error.is_some_and(|err| !is_hidden_by_member(err) && !is_http_status(err, 404))
}

/// The pager shows when this page has rows, and on any later page even when it
/// failed to load, so Previous stays reachable.
fn show_pager(has_rows: bool, page_number: usize) -> bool {
    has_rows || page_number > 1
}

/// Text of the always-rendered status line. From page 2 on it names the page,
/// so it changes on every page load and a screen reader announces it.
fn status_text(
    loading: bool,
    not_found: bool,
    error: Option<&str>,
    rows: Option<usize>,
    page_number: usize,
) -> String {
    if not_found || error.is_some_and(|err| is_http_status(err, 404)) {
        return NOT_FOUND_COPY.to_string();
    }
    if let Some(err) = error {
        if is_hidden_by_member(err) {
            return HIDDEN_COPY.to_string();
        }
        // Keep the server's reason after the plain sentence.
        let reason = err.trim();
        let lead = if page_number > 1 {
            format!("Couldn't load page {page_number} of matches.")
        } else {
            "Couldn't load matches.".to_string()
        };
        return if reason.is_empty() {
            lead
        } else {
            format!("{lead} {reason}")
        };
    }
    let rows = if loading { None } else { rows };
    match (rows, page_number > 1) {
        (None, false) => "Loading matches.".to_string(),
        (None, true) => format!("Loading page {page_number}."),
        (Some(0), false) => "No matches tracked for this member yet.".to_string(),
        (Some(0), true) => format!("No matches on page {page_number}."),
        (Some(1), false) => "Showing 1 match.".to_string(),
        (Some(1), true) => format!("Page {page_number}: showing 1 match."),
        (Some(n), false) => format!("Showing {n} matches."),
        (Some(n), true) => format!("Page {page_number}: showing {n} matches."),
    }
}

/// Characters of `raw` that are safe in an element id, with every other one
/// turned into an underscore. Ids come from data the server parsed, never
/// straight from the URL, and are deterministic so server rendering and
/// hydration agree. Mapping to _ could collide (a/b vs a_b), but real record
/// ids are plain letters and digits, so they cannot.
fn dom_key(raw: &str) -> String {
    let key: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if key.is_empty() {
        "member".to_string()
    } else {
        key
    }
}

/// Screen reader and tooltip text for a flagged cell, for example "Damage might
/// be misread".
fn unsure_text(stat_name: &str) -> String {
    format!("{stat_name} might be misread")
}

/// `Victory` for the stored `victory`. Unknown text keeps its own letters.
fn outcome_label(outcome: &str) -> String {
    let mut chars = outcome.trim().chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "-".to_string(),
    }
}

fn cell_is_unsure(fields: &[String], name: &str) -> bool {
    scuffed_types::SUSPECT_FIELD_NAMES.contains(&name) && fields.iter().any(|f| f == name)
}

/// A number or text cell. A flagged cell gets a mark, a hover title, and hidden
/// "{stat} might be misread" text, so each mark has its own readable name.
fn value_cell(text: String, unsure: bool, stat_name: &str) -> Element {
    if unsure {
        let note = unsure_text(stat_name);
        rsx! {
            span { class: "mm-unsure", title: "{note}",
                "{text}"
                span { class: "mm-unsure-mark", aria_hidden: "true", "?" }
                span { class: "visually-hidden", "{note}" }
            }
        }
    } else {
        rsx! { "{text}" }
    }
}

fn cell_or_dash(text: &str) -> String {
    if text.trim().is_empty() {
        "-".to_string()
    } else {
        text.to_string()
    }
}

const MEMBER_MATCHES_CSS: &str = r#"
    /* The visually hidden note is absolutely positioned. Without a positioned
       parent it escapes the table's scroll box and widens the whole page. */
    .mm-unsure { position: relative; }
    .mm-unsure-mark {
        margin-inline-start: var(--space-1);
        color: var(--warn);
        font-size: var(--text-sm);
        font-weight: var(--font-label-weight);
    }
    .mm-totals {
        display: flex;
        flex-direction: column;
        gap: var(--space-stack);
        min-width: 0;
    }
    /* Phone cards: the five stats share a line ("Elims: 10  Deaths: 3 ..."),
       wrapping as needed, instead of one labelled line each. */
    @media (max-width: 820px) {
        .admin-member-matches-page .is-cards .data-table td.mm-stat {
            flex: 0 0 auto;
            margin-inline-end: var(--space-4);
        }
    }
"#;

/// `10000` as `10,000`: big stat values grouped in thousands.
fn group_thousands(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// The list region: the pager's focus target after a page change.
const LIST_ID: &str = "admin-member-matches-list";

/// Tone of a result pill. The pill always carries the word.
fn outcome_tone(outcome: &str) -> PillTone {
    match outcome.trim().to_ascii_lowercase().as_str() {
        "victory" | "win" => PillTone::Ok,
        "defeat" | "loss" => PillTone::Danger,
        _ => PillTone::Neutral,
    }
}

/// `1 Nov 2026, 18:30 UTC`: the date the admin lists write, plus the time.
fn played_label(at: &DateTime<Utc>) -> String {
    at.format("%-d %b %Y, %H:%M UTC").to_string()
}

/// Class of a pager step: the shared pager's look, `is-disabled` at an end.
fn step_class(off: bool) -> &'static str {
    if off {
        "pager-step is-disabled"
    } else {
        "pager-step"
    }
}

/// The route component. The body is keyed on the id, so a change of `:id` in
/// the URL mounts a fresh body: paging, loaded rows and totals reset instead of
/// showing the previous member under the new address.
#[component]
pub fn AdminMemberMatches(id: String) -> Element {
    rsx! { MemberMatchesBody { key: "{id}", id } }
}

#[component]
fn MemberMatchesBody(id: String) -> Element {
    let mut page_cursor = use_signal(|| Option::<String>::None);
    let mut cursor_history = use_signal(|| vec![Option::<String>::None]);

    let name_id = id.clone();
    let totals_id = id.clone();
    let matches_id = id.clone();
    let name = use_api_with::<MemberName>(move || name_path(&name_id));
    let totals = use_api_with::<Totals>(move || totals_path(&totals_id));
    let matches =
        use_api_with::<MatchPage>(move || matches_path(&matches_id, page_cursor().as_deref()));

    let who = name
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .map(|m| (m.id.clone(), m.display_name.clone()));
    // Element ids use the record id the server returned, not the URL segment.
    let key = who
        .as_ref()
        .map(|(record_id, _)| dom_key(record_id))
        .unwrap_or_else(|| "member".to_string());

    let error = matches.error.read().clone();
    let name_error = name.error.read().clone();
    let not_found = is_not_found(error.as_deref(), name_error.as_deref());
    let heading = match (&who, not_found) {
        (_, true) => "Member not found".to_string(),
        (Some((_, n)), false) => format!("Matches for {n}"),
        (None, false) => "Member matches".to_string(),
    };
    let rows = matches
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .map(|p| p.data.len());
    let loading = error.is_none() && rows.is_none();
    let page_number = cursor_history().len();
    let can_prev = page_number > 1;
    let status = status_text(loading, not_found, error.as_deref(), rows, page_number);
    let status_id = format!("member-matches-status-{key}");
    let totals_heading_id = format!("member-matches-totals-heading-{key}");
    let retry = !not_found && can_retry(error.as_deref());
    let pager = show_pager(rows.is_some_and(|n| n > 0), page_number);
    let next = matches
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .and_then(|p| p.next_cursor.clone());

    let totals_error = totals.error.read().clone();
    let mut totals_refresh = totals.refresh;
    let mut matches_refresh = matches.refresh;
    let empty_first_page = rows == Some(0) && page_number == 1 && error.is_none();

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-admin-member_matches-css", css: MEMBER_MATCHES_CSS }
        PageShell { width: PageWidth::Wide, class: "admin-member-matches-page",
            PageHeader {
                title: heading,
                lead: (!not_found).then(|| "Single matches from the stat tracker, newest first. A question mark marks a value that might be misread.".to_string()),
                actions: rsx! {
                    Link { class: "btn is-ghost", to: Route::AdminMembers {}, "Back to members" }
                },
            }

            if !not_found {
                section { class: "mm-totals", aria_labelledby: "{totals_heading_id}",
                    h2 { id: "{totals_heading_id}", class: "ui-section-title", "Totals" }
                    {
                        let data = totals.data.read();
                        match data.as_ref().and_then(|d| d.as_ref()) {
                            Some(t) => rsx! {
                                div { class: "summary-cards",
                                    SummaryCard { value: t.total_matches.to_string(), label: "Matches" }
                                    SummaryCard { value: t.wins.to_string(), label: "Wins" }
                                    SummaryCard { value: t.losses.to_string(), label: "Losses" }
                                    SummaryCard { value: t.draws.to_string(), label: "Draws" }
                                }
                            },
                            None => match totals_error.clone() {
                                Some(err) => rsx! {
                                    div { class: "fetch-error-wrap", role: "alert",
                                        p { class: "fetch-error", "Couldn't load totals. {err}" }
                                        button {
                                            r#type: "button",
                                            class: "fetch-error__retry",
                                            onclick: move |_| totals_refresh += 1,
                                            "Retry"
                                        }
                                    }
                                },
                                None => rsx! {
                                    p { class: "admin-pending-text", role: "status", "Loading totals..." }
                                    SkeletonBars { rows: 1 }
                                },
                            },
                        }
                    }
                }
            }

            section {
                id: LIST_ID,
                class: "admin-list pager-target",
                tabindex: "-1",
                aria_label: "Matches",
                // Always rendered: every state of the list is said here.
                p { class: "admin-pending-text", id: "{status_id}", role: "status", "{status}" }
                if loading {
                    SkeletonBars { rows: 3 }
                }
                if retry {
                    button {
                        class: "fetch-error__retry",
                        r#type: "button",
                        onclick: move |_| matches_refresh += 1,
                        "Retry loading matches"
                    }
                }
                if not_found {
                    EmptyCard {
                        title: "Nothing to show",
                        message: "Check the link, or pick a member from the member list.",
                        actions: rsx! {
                            Link { class: "btn is-primary", to: Route::AdminMembers {}, "Open members" }
                        },
                    }
                } else if empty_first_page {
                    EmptyCard {
                        title: "No matches yet",
                        message: "Matches show up here once this member's stat tracker uploads a game.",
                        actions: rsx! {
                            Link { class: "btn is-ghost", to: Route::AdminMembers {}, "Back to members" }
                        },
                    }
                }
                {
                    let data = matches.data.read();
                    match data.as_ref().and_then(|d| d.as_ref()) {
                        Some(page) if !page.data.is_empty() => rsx! {
                            MatchesTable { rows: page.data.clone() }
                        },
                        _ => rsx! {},
                    }
                }

                // The pager stays on a later page even when that page failed to
                // load, so Previous is always reachable. Next needs the cursor of
                // a loaded page. The API has no total, so it steps by cursor.
                if pager {
                    div { id: "member-matches-pager", class: "pager",
                        p { class: "pager-meta", "Page {page_number}" }
                        nav { class: "pager-pages", aria_label: "Pages of matches",
                            button {
                                r#type: "button",
                                id: "member-matches-pager-prev",
                                class: step_class(!can_prev),
                                aria_label: "Previous page of matches",
                                aria_disabled: if !can_prev { "true" },
                                onclick: move |_| {
                                    let mut hist = cursor_history.write();
                                    if hist.len() > 1 {
                                        hist.pop();
                                        let prev = hist.last().cloned().flatten();
                                        drop(hist);
                                        page_cursor.set(prev);
                                        show_list_top(LIST_ID);
                                    }
                                },
                                "Previous"
                            }
                            button {
                                r#type: "button",
                                id: "member-matches-pager-next",
                                class: step_class(next.is_none()),
                                aria_label: "Next page of matches",
                                aria_disabled: if next.is_none() { "true" },
                                onclick: move |_| {
                                    if let Some(c) = next.clone() {
                                        cursor_history.write().push(Some(c.clone()));
                                        page_cursor.set(Some(c));
                                        show_list_top(LIST_ID);
                                    }
                                },
                                "Next"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// One page of matches. On phone every row is a card: hero and the result
/// on top, map, role and time on one line, then one line per stat.
#[component]
fn MatchesTable(rows: Vec<MemberMatch>) -> Element {
    rsx! {
        DataTable {
            headers: vec!["Hero", "Result", "Map", "Role", "Played", "Elims", "Deaths", "Assists", "Damage", "Healing"],
            num_cols: vec![4, 5, 6, 7, 8, 9],
            cards: true,
            label: "Matches",
            for m in rows.iter() {
                Fragment { key: "{m.id}", {
                    let fields = m.suspect_fields.as_deref().unwrap_or(&[]);
                    let played = played_label(&m.played_at);
                    let played_iso = m.played_at.to_rfc3339();
                    let row_id = format!("member-match-{}", dom_key(&m.id));
                    rsx! {
                        tr { key: "{m.id}", id: "{row_id}",
                            td { class: "cell-who", {value_cell(cell_or_dash(&m.hero), cell_is_unsure(fields, "hero"), "Hero")} }
                            td { class: "cell-badge",
                                Pill { tone: outcome_tone(&m.outcome),
                                    {value_cell(outcome_label(&m.outcome), cell_is_unsure(fields, "result"), "Result")}
                                }
                            }
                            td { class: "cell-meta", {value_cell(cell_or_dash(&m.map_name), cell_is_unsure(fields, "map"), "Map")} }
                            td { class: "cell-meta", "{cell_or_dash(&m.role)}" }
                            td { class: "cell-meta num",
                                time { class: "date", datetime: "{played_iso}", "{played}" }
                            }
                            td { class: "num mm-stat", "data-label": "Elims", {value_cell(group_thousands(m.elims), cell_is_unsure(fields, "e"), "Eliminations")} }
                            td { class: "num mm-stat", "data-label": "Deaths", {value_cell(group_thousands(m.deaths), cell_is_unsure(fields, "d"), "Deaths")} }
                            td { class: "num mm-stat", "data-label": "Assists", {value_cell(group_thousands(m.assists), cell_is_unsure(fields, "a"), "Assists")} }
                            td { class: "num mm-stat", "data-label": "Damage", {value_cell(group_thousands(m.damage), cell_is_unsure(fields, "dmg"), "Damage")} }
                            td { class: "num mm-stat", "data-label": "Healing", {value_cell(group_thousands(m.healing), cell_is_unsure(fields, "h"), "Healing")} }
                        }
                    }
                } }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_stats_are_grouped_in_thousands() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(6000), "6,000");
        assert_eq!(group_thousands(10000), "10,000");
        assert_eq!(group_thousands(1234567), "1,234,567");
    }

    #[test]
    fn paths_use_the_601_routes() {
        assert_eq!(
            matches_path("abc", None),
            "/api/stats/member/abc/matches?limit=25"
        );
        assert_eq!(
            matches_path("abc", Some("00000025")),
            "/api/stats/member/abc/matches?limit=25&cursor=00000025"
        );
        assert_eq!(totals_path("abc"), "/api/stats/member/abc");
        assert_eq!(name_path("abc"), "/api/members/abc");
        assert!(matches_path("a/b", None).contains("a%2Fb"));
    }

    #[test]
    fn hidden_by_member_is_the_403_only() {
        assert!(is_hidden_by_member("HTTP error 403: hidden_by_member"));
        assert!(!is_hidden_by_member(
            "HTTP error 403: Officer access required"
        ));
        assert!(!is_hidden_by_member("HTTP error 404: Member not found"));
        assert!(!is_hidden_by_member("HTTP error 500: hidden_by_member"));
    }

    #[test]
    fn hidden_match_uses_the_shared_constant() {
        let shared = format!("HTTP error 403: {STATS_HIDDEN_BY_MEMBER}");
        assert!(is_hidden_by_member(&shared));
        assert_eq!(STATS_HIDDEN_BY_MEMBER, "hidden_by_member");
        let source = include_str!("member_matches.rs");
        let typed_copy = format!("contains(\"{}\")", "hidden_by_member");
        assert!(
            !source.contains(&typed_copy),
            "match on STATS_HIDDEN_BY_MEMBER, not a typed copy"
        );
    }

    #[test]
    fn not_found_is_a_404_from_either_request() {
        let nf = "HTTP error 404: Member not found";
        assert!(is_not_found(Some(nf), None));
        assert!(is_not_found(None, Some(nf)));
        assert!(is_not_found(Some("HTTP error 500: x"), Some(nf)));
        assert!(!is_not_found(None, None));
        assert!(!is_not_found(
            Some("HTTP error 403: hidden_by_member"),
            None
        ));
        assert!(!is_not_found(Some("HTTP error 500: x"), None));
    }

    #[test]
    fn retry_only_for_a_failure_that_can_pass() {
        assert!(can_retry(Some("HTTP error 500: Internal error")));
        assert!(can_retry(Some("network error")));
        assert!(!can_retry(None));
        assert!(!can_retry(Some("HTTP error 404: Member not found")));
        assert!(!can_retry(Some("HTTP error 403: hidden_by_member")));
    }

    #[test]
    fn pager_stays_on_a_later_page_that_failed() {
        assert!(show_pager(true, 1));
        assert!(!show_pager(false, 1));
        assert!(
            show_pager(false, 2),
            "a failed page 2 keeps Previous reachable"
        );
    }

    #[test]
    fn status_text_covers_every_state() {
        assert_eq!(status_text(true, false, None, None, 1), "Loading matches.");
        assert_eq!(
            status_text(
                false,
                false,
                Some("HTTP error 403: hidden_by_member"),
                None,
                1
            ),
            HIDDEN_COPY
        );
        assert_eq!(
            status_text(false, true, None, None, 1),
            "This member was not found."
        );
        assert_eq!(
            status_text(false, false, Some("HTTP error 404: x"), None, 1),
            "This member was not found."
        );
        assert_eq!(
            status_text(
                false,
                false,
                Some("HTTP error 500: Internal error"),
                None,
                1
            ),
            "Couldn't load matches. HTTP error 500: Internal error"
        );
        assert_eq!(
            status_text(false, false, None, Some(0), 1),
            "No matches tracked for this member yet."
        );
        assert_eq!(
            status_text(false, false, None, Some(1), 1),
            "Showing 1 match."
        );
        assert_eq!(
            status_text(false, false, None, Some(25), 1),
            "Showing 25 matches."
        );
    }

    #[test]
    fn status_text_changes_on_every_page_load() {
        let one = status_text(false, false, None, Some(25), 1);
        let two = status_text(false, false, None, Some(25), 2);
        let three = status_text(false, false, None, Some(25), 3);
        assert_eq!(two, "Page 2: showing 25 matches.");
        assert_ne!(one, two, "paging page 1 to 2 changes the text");
        assert_ne!(two, three);
        assert_eq!(status_text(true, false, None, None, 2), "Loading page 2.");
        assert_eq!(
            status_text(false, false, Some("HTTP error 500: x"), None, 2),
            "Couldn't load page 2 of matches. HTTP error 500: x"
        );
        assert_eq!(
            status_text(false, false, None, Some(1), 2),
            "Page 2: showing 1 match."
        );
    }

    #[test]
    fn copy_has_no_dashes() {
        for text in [
            HIDDEN_COPY,
            NOT_FOUND_COPY,
            &status_text(false, false, None, Some(3), 1),
            &status_text(false, false, None, Some(3), 2),
            &status_text(true, false, None, None, 2),
            &status_text(false, false, Some("HTTP error 500: x"), None, 2),
            &unsure_text("Damage"),
        ] {
            assert!(
                !text.contains('\u{2014}') && !text.contains('\u{2013}'),
                "{text}"
            );
        }
    }

    #[test]
    fn dom_keys_are_id_safe_and_never_empty() {
        assert_eq!(dom_key("abc123"), "abc123");
        assert_eq!(dom_key("a-b_c"), "a-b_c");
        assert_eq!(dom_key("a b\"c/d?e=f"), "a_b_c_d_e_f");
        assert_eq!(dom_key("member:xyz"), "member_xyz");
        assert_eq!(dom_key(""), "member");
        assert!(dom_key("<script>").chars().all(|c| c != '<' && c != '>'));
    }

    #[test]
    fn member_name_carries_the_record_id() {
        let name: MemberName =
            serde_json::from_str(r#"{"id":"abc","display_name":"Sample A","role":"member"}"#)
                .unwrap();
        assert_eq!(name.id, "abc");
        assert_eq!(name.display_name, "Sample A");
    }

    #[test]
    fn unsure_text_names_the_stat_and_drops_the_tracker_hint() {
        assert_eq!(unsure_text("Damage"), "Damage might be misread");
        assert!(!unsure_text("Damage").contains("tracker"));
        assert!(!unsure_text("Damage").contains("unsure"));
    }

    #[test]
    fn flagged_cell_has_title_and_named_screen_reader_text() {
        fn view() -> Element {
            rsx! {
                {value_cell("4000".to_string(), true, "Damage")}
                {value_cell("12".to_string(), false, "Deaths")}
            }
        }
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains("title=\"Damage might be misread\""), "{html}");
        assert!(
            html.contains("<span class=\"visually-hidden\">Damage might be misread</span>"),
            "{html}"
        );
        assert!(!html.contains("(unsure)"), "{html}");
        assert!(!html.contains("Check it in the tracker"), "{html}");
        assert_eq!(html.matches("mm-unsure-mark").count(), 1, "{html}");
        assert!(html.contains("12"), "{html}");
        assert!(!html.contains("Deaths might be misread"), "{html}");
    }

    #[test]
    fn hidden_note_is_anchored_inside_its_cell() {
        let rule = MEMBER_MATCHES_CSS
            .split("\n")
            .find(|l| l.trim_start().starts_with(".mm-unsure {"))
            .expect("mm-unsure rule");
        assert!(rule.contains("position: relative"), "{rule}");
        // The note uses the shared visually-hidden class, which is absolute.
        let common = crate::styles::common::CSS;
        let hidden = &common[common.find(".visually-hidden {").expect("shared rule")..];
        assert!(hidden[..hidden.find('}').unwrap()].contains("position: absolute"));
        crate::components::ui::test_support::assert_sizes_from_tokens(MEMBER_MATCHES_CSS);
    }

    #[test]
    fn page_has_one_h1_a_back_link_and_an_always_rendered_status_line() {
        use crate::components::admin_shared::test_page::render_admin_route;
        let html = render_admin_route(
            "/admin/members/m1/matches",
            Some(scuffed_types::OrgRole::Officer),
        );
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide admin-member-matches-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Member matches</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(html.contains(">Back to members</a>"), "{html}");
        assert!(
            html.contains(
                "<p class=\"admin-pending-text\" id=\"member-matches-status-member\" role=\"status\">Loading matches.</p>"
            ),
            "{html}"
        );
        assert!(
            html.contains("role=\"status\">Loading totals...</p>"),
            "{html}"
        );
    }

    #[test]
    fn rows_are_cards_with_a_result_pill_and_labelled_stats() {
        fn view() -> Element {
            let row: MemberMatch = serde_json::from_str(
                r#"{"id":"m1","hero":"Ana","map_name":"Ilios","role":"Support","outcome":"defeat","elims":10,"deaths":2,"assists":5,"damage":4000,"healing":9000,"played_at":"2026-11-01T18:30:00Z","suspect_fields":["dmg"]}"#,
            )
            .unwrap();
            rsx! { MatchesTable { rows: vec![row] } }
        }
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains("data-table-scroll is-cards"), "{html}");
        assert!(
            html.contains("<th scope=\"col\" class=\"num\">Damage</th>"),
            "{html}"
        );
        assert!(
            html.contains("class=\"ui-pill ui-pill--danger\">Defeat</span>"),
            "{html}"
        );
        assert!(
            html.contains("<time class=\"date\" datetime=\"2026-11-01T18:30:00+00:00\">1 Nov 2026, 18:30 UTC</time>"),
            "{html}"
        );
        assert!(html.contains("data-label=\"Healing\""), "{html}");
        assert!(html.contains("Damage might be misread"), "{html}");
        assert_eq!(outcome_tone("victory"), PillTone::Ok);
        assert_eq!(outcome_tone("Draw"), PillTone::Neutral);
    }

    #[test]
    fn outcome_label_capitalises() {
        assert_eq!(outcome_label("victory"), "Victory");
        assert_eq!(outcome_label("defeat"), "Defeat");
        assert_eq!(outcome_label(""), "-");
    }

    #[test]
    fn page_parses_the_601_shape() {
        let json = r#"{"data":[{"id":"m1","member_id":"x","session_id":"","hero":"Ana","map_name":"Ilios","game_mode":"Control","role":"Support","outcome":"victory","elims":10,"deaths":2,"assists":5,"damage":4000,"healing":9000,"mitigation":0,"played_at":"2026-11-01T18:30:00Z","uploaded_at":"2026-11-01T18:40:00Z","edited":false,"recognizer":"ocr-v1","name_recognizer":null,"suspect_fields":["dmg"]}],"next_cursor":"25"}"#;
        let page: MatchPage = serde_json::from_str(json).unwrap();
        assert_eq!(page.data.len(), 1);
        assert_eq!(page.next_cursor.as_deref(), Some("25"));
        let fields = page.data[0].suspect_fields.as_deref().unwrap();
        assert!(cell_is_unsure(fields, "dmg"));
        assert!(!cell_is_unsure(fields, "e"));
        let last: MatchPage = serde_json::from_str(r#"{"data":[]}"#).unwrap();
        assert!(last.next_cursor.is_none());
    }
}
