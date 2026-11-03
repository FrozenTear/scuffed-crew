//! Officer view of one member's single matches.
//!
//! Reads `GET /api/stats/member/{id}/matches` (officer only). A member whose
//! `stats_visibility` is `private` answers 403 `hidden_by_member`. The page then
//! says so calmly and still shows the totals from `GET /api/stats/member/{id}`,
//! which officers may read for every visibility. An officer's own id always loads.

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::{DataTable, is_http_status};
use crate::hooks::use_api_with;
use crate::routes::Route;
use crate::util::encode_query;

/// Rows per page. The server accepts 1 to 100.
const PAGE_LIMIT: u32 = 25;

/// Calm copy for a member who hides single matches. Totals stay visible.
pub(crate) const HIDDEN_COPY: &str =
    "This member keeps their single matches private. Their totals are still shown below.";

#[derive(Debug, Clone, Deserialize)]
struct MemberName {
    display_name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Totals {
    total_matches: u32,
    wins: u32,
    losses: u32,
    draws: u32,
}

#[derive(Debug, Clone, Deserialize)]
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

/// True when a `use_api*` error string is the 403 for a member who hides matches.
fn is_hidden_by_member(error: &str) -> bool {
    is_http_status(error, 403) && error.contains("hidden_by_member")
}

/// Text of the always-rendered status line.
fn status_text(loading: bool, error: Option<&str>, rows: Option<usize>) -> String {
    if let Some(err) = error {
        if is_hidden_by_member(err) {
            return HIDDEN_COPY.to_string();
        }
        if is_http_status(err, 404) {
            return "This member was not found.".to_string();
        }
        return "Couldn't load matches.".to_string();
    }
    match rows {
        _ if loading => "Loading matches.".to_string(),
        None => "Loading matches.".to_string(),
        Some(0) => "No matches tracked for this member yet.".to_string(),
        Some(1) => "Showing 1 match.".to_string(),
        Some(n) => format!("Showing {n} matches."),
    }
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

/// A number or text cell. A flagged cell gets a mark and hidden "(unsure)" text.
fn value_cell(text: String, unsure: bool) -> Element {
    if unsure {
        rsx! {
            span { class: "mm-unsure",
                "{text}"
                span { class: "mm-unsure-mark", aria_hidden: "true", "?" }
                span { class: "mm-sr-only", "(unsure)" }
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
    .mm-back { display: inline-block; margin-bottom: 1rem; color: var(--accent); }
    .mm-status { color: var(--text-2); margin: 0 0 1rem; min-height: 1.5em; }
    .mm-totals { margin: 0 0 1.5rem; }
    .mm-totals h2 {
        font-family: var(--font-head);
        font-size: 1rem;
        text-transform: uppercase;
        letter-spacing: 0.06em;
        color: var(--text-3);
        margin: 0 0 0.5rem;
    }
    .mm-totals dl { display: flex; flex-wrap: wrap; gap: 0.75rem 2rem; margin: 0; }
    .mm-totals dt { font-size: 0.75rem; color: var(--text-3); text-transform: uppercase; }
    .mm-totals dd { margin: 0; font-family: var(--font-head); font-size: 1.4rem; color: var(--text); }
    .mm-unsure-mark { margin-left: 0.15rem; color: var(--warn); }
    .mm-sr-only {
        position: absolute;
        width: 1px;
        height: 1px;
        margin: -1px;
        padding: 0;
        overflow: hidden;
        clip: rect(0, 0, 0, 0);
        white-space: nowrap;
        border: 0;
    }
"#;

#[component]
pub fn AdminMemberMatches(id: String) -> Element {
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
        .map(|m| m.display_name.clone());
    let heading = match &who {
        Some(n) => format!("Matches for {n}"),
        None => "Member matches".to_string(),
    };

    let error = matches.error.read().clone();
    let hidden = error.as_deref().is_some_and(is_hidden_by_member);
    let rows = matches
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .map(|p| p.data.len());
    let loading = error.is_none() && rows.is_none();
    let status = status_text(loading, error.as_deref(), rows);
    let status_id = format!("member-matches-status-{id}");
    let totals_heading_id = format!("member-matches-totals-heading-{id}");
    let can_retry = error.is_some() && !hidden;
    let page_number = cursor_history().len();
    let can_prev = page_number > 1;

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-admin-member_matches-css", css: MEMBER_MATCHES_CSS }

        Link { class: "mm-back", to: Route::AdminMembers {}, "Back to members" }
        div { class: "admin-toolbar",
            h1 { "{heading}" }
        }

        p { class: "mm-status", id: "{status_id}", role: "status", "{status}" }
        if can_retry {
            {
                let mut refresh = matches.refresh;
                rsx! {
                    button {
                        class: "row-btn",
                        r#type: "button",
                        onclick: move |_| refresh += 1,
                        "Retry loading matches"
                    }
                }
            }
        }

        section { class: "mm-totals", aria_labelledby: "{totals_heading_id}",
            h2 { id: "{totals_heading_id}", "Totals" }
            {
                let data = totals.data.read();
                match data.as_ref().and_then(|d| d.as_ref()) {
                    Some(t) => rsx! {
                        dl {
                            div {
                                dt { "Matches" }
                                dd { id: "member-matches-total-{id}", "{t.total_matches}" }
                            }
                            div {
                                dt { "Wins" }
                                dd { id: "member-matches-wins-{id}", "{t.wins}" }
                            }
                            div {
                                dt { "Losses" }
                                dd { id: "member-matches-losses-{id}", "{t.losses}" }
                            }
                            div {
                                dt { "Draws" }
                                dd { id: "member-matches-draws-{id}", "{t.draws}" }
                            }
                        }
                    },
                    None if totals.error.read().is_some() => rsx! {
                        p { class: "empty-state", "Couldn't load totals." }
                    },
                    None => rsx! { p { class: "admin-loading", "Loading totals." } },
                }
            }
        }

        {
            let data = matches.data.read();
            match data.as_ref().and_then(|d| d.as_ref()) {
                Some(page) if !page.data.is_empty() => {
                    let next = page.next_cursor.clone();
                    rsx! {
                        DataTable {
                            headers: vec!["Played", "Result", "Hero", "Map", "Role", "Elims", "Deaths", "Assists", "Damage", "Healing"],
                            for m in page.data.iter() {
                                {
                                    let fields = m.suspect_fields.as_deref().unwrap_or(&[]);
                                    let played = m.played_at.format("%Y-%m-%d %H:%M UTC").to_string();
                                    rsx! {
                                        tr { key: "{m.id}", id: "member-match-{m.id}",
                                            td { "{played}" }
                                            td { {value_cell(outcome_label(&m.outcome), cell_is_unsure(fields, "result"))} }
                                            td { {value_cell(cell_or_dash(&m.hero), cell_is_unsure(fields, "hero"))} }
                                            td { {value_cell(cell_or_dash(&m.map_name), cell_is_unsure(fields, "map"))} }
                                            td { "{cell_or_dash(&m.role)}" }
                                            td { {value_cell(m.elims.to_string(), cell_is_unsure(fields, "e"))} }
                                            td { {value_cell(m.deaths.to_string(), cell_is_unsure(fields, "d"))} }
                                            td { {value_cell(m.assists.to_string(), cell_is_unsure(fields, "a"))} }
                                            td { {value_cell(m.damage.to_string(), cell_is_unsure(fields, "dmg"))} }
                                            td { {value_cell(m.healing.to_string(), cell_is_unsure(fields, "h"))} }
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "pagination",
                            button {
                                r#type: "button",
                                aria_label: "Previous page of matches",
                                disabled: !can_prev,
                                onclick: move |_| {
                                    let mut hist = cursor_history.write();
                                    if hist.len() > 1 {
                                        hist.pop();
                                        let prev = hist.last().cloned().flatten();
                                        drop(hist);
                                        page_cursor.set(prev);
                                    }
                                },
                                "Previous"
                            }
                            span { class: "page-info", "Page {page_number}" }
                            button {
                                r#type: "button",
                                aria_label: "Next page of matches",
                                disabled: next.is_none(),
                                onclick: move |_| {
                                    if let Some(c) = next.clone() {
                                        cursor_history.write().push(Some(c.clone()));
                                        page_cursor.set(Some(c));
                                    }
                                },
                                "Next"
                            }
                        }
                    }
                }
                _ => rsx! {},
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn status_text_covers_every_state() {
        assert_eq!(status_text(true, None, None), "Loading matches.");
        assert_eq!(
            status_text(false, Some("HTTP error 403: hidden_by_member"), None),
            HIDDEN_COPY
        );
        assert_eq!(
            status_text(false, Some("HTTP error 404: Member not found"), None),
            "This member was not found."
        );
        assert_eq!(
            status_text(false, Some("HTTP error 500: Internal error"), None),
            "Couldn't load matches."
        );
        assert_eq!(
            status_text(false, None, Some(0)),
            "No matches tracked for this member yet."
        );
        assert_eq!(status_text(false, None, Some(1)), "Showing 1 match.");
        assert_eq!(status_text(false, None, Some(25)), "Showing 25 matches.");
    }

    #[test]
    fn copy_has_no_dashes_or_banned_words() {
        for text in [
            HIDDEN_COPY,
            &status_text(false, None, Some(3)),
            &status_text(false, Some("HTTP error 404: x"), None),
        ] {
            assert!(
                !text.contains('\u{2014}') && !text.contains('\u{2013}'),
                "{text}"
            );
            assert!(!text.to_lowercase().contains("streamer"), "{text}");
        }
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
