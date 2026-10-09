use dioxus::prelude::*;

use crate::hooks::ApiResource;

use super::role::{history_row_matches_role, stored_role_label};
use super::{MatchPage, format_date, load_error_state};

/// The tracker stores outcomes as `victory` / `defeat` / `draw` (see the
/// upload filter in site-server routes/stats.rs); `win` / `loss` are accepted
/// as aliases. DEFEAT must land on the serious/danger class — before this
/// matched only `win`/`loss`, every card fell through to the draw class and
/// DEFEAT rendered gold.
fn outcome_class(outcome: &str) -> &'static str {
    match outcome.to_lowercase().as_str() {
        "victory" | "win" => "outcome-win",
        "defeat" | "loss" => "outcome-loss",
        _ => "outcome-draw",
    }
}

/// Visible label when a row was not read by the old Tesseract reader.
const NEW_READER_BADGE: &str = "New reader (alpha)";
/// Tooltip for [`NEW_READER_BADGE`].
const NEW_READER_TITLE: &str = "Read by the new stat reader, still in testing";
/// Title and accessible name on a cell the reader flagged.
const UNSURE_TEXT: &str = "The reader was unsure about this value";

/// `true` when this row should show the new-reader badge.
pub(super) fn show_new_reader_badge(recognizer: &str) -> bool {
    recognizer != scuffed_types::RECOGNIZER_OCR_V1
}

/// `true` when `name` is a known match cell and this row flagged it.
/// Names outside that set are ignored.
pub(super) fn cell_is_unsure(suspect_fields: &[String], name: &str) -> bool {
    scuffed_types::SUSPECT_FIELD_NAMES.contains(&name)
        && suspect_fields.iter().any(|field| field == name)
}

fn unsure_mark() -> Element {
    rsx! {
        span {
            class: "stat-unsure-mark",
            title: UNSURE_TEXT,
            aria_label: UNSURE_TEXT,
            "?"
        }
    }
}

/// One match-row value. An unsure cell gets a question mark plus the unsure title.
fn marked_value(class: &str, text: &str, fallback_title: Option<&str>, unsure: bool) -> Element {
    let class = if unsure {
        if class.is_empty() {
            "stat-unsure".to_string()
        } else {
            format!("{class} stat-unsure")
        }
    } else {
        class.to_string()
    };
    let text = text.to_string();
    if unsure {
        rsx! {
            span {
                class: "{class}",
                title: UNSURE_TEXT,
                "{text}"
                {unsure_mark()}
            }
        }
    } else if let Some(title) = fallback_title {
        let title = title.to_string();
        rsx! {
            span {
                class: "{class}",
                title: "{title}",
                "{text}"
            }
        }
    } else if class.is_empty() {
        rsx! { span { "{text}" } }
    } else {
        rsx! { span { class: "{class}", "{text}" } }
    }
}

pub(super) fn history_tab(
    matches: ApiResource<MatchPage>,
    mut page_cursor: Signal<Option<String>>,
    mut cursor_history: Signal<Vec<Option<String>>>,
    mut outcome_filter: Signal<&'static str>,
    mut role_filter: Signal<&'static str>,
) -> Element {
    let err = matches.error.read().clone();
    let data = matches.data.read();
    let page = data.as_ref().and_then(|d| d.as_ref());
    match page {
        None if err.is_some() => load_error_state("match history", matches.refresh),
        None => rsx! { p { class: "loading-state", "Loading match history..." } },
        Some(page) if page.data.is_empty() => rsx! {
            p { class: "empty-state", "No matches recorded yet." }
        },
        Some(page) => {
            let has_next = page.next_cursor.is_some();
            let next_c = page.next_cursor.clone();
            let can_prev = cursor_history().len() > 1;
            let of = outcome_filter();
            let rf = role_filter();

            let rows: Vec<_> = page
                .data
                .iter()
                .filter(|m| {
                    let o_ok = match of {
                        "all" => true,
                        "win" => {
                            m.outcome.eq_ignore_ascii_case("win")
                                || m.outcome.eq_ignore_ascii_case("victory")
                        }
                        "loss" => {
                            m.outcome.eq_ignore_ascii_case("loss")
                                || m.outcome.eq_ignore_ascii_case("defeat")
                        }
                        "draw" => {
                            !m.outcome.eq_ignore_ascii_case("win")
                                && !m.outcome.eq_ignore_ascii_case("victory")
                                && !m.outcome.eq_ignore_ascii_case("loss")
                                && !m.outcome.eq_ignore_ascii_case("defeat")
                        }
                        _ => m.outcome.eq_ignore_ascii_case(of),
                    };
                    let r_ok = history_row_matches_role(&m.role, rf);
                    o_ok && r_ok
                })
                .collect();

            rsx! {
                div { class: "stats-filters",
                    div { class: "filter-group",
                        span { class: "filter-label", "Outcome" }
                        {[["all", "All"], ["win", "Win"], ["loss", "Loss"], ["draw", "Draw"]].iter().map(|&[k, label]| {
                            let active = of == k;
                            rsx! {
                                button {
                                    key: "{k}",
                                    class: if active { "filter-chip active" } else { "filter-chip" },
                                    onclick: move |_| outcome_filter.set(k),
                                    "{label}"
                                }
                            }
                        })}
                    }
                    div { class: "filter-group",
                        span { class: "filter-label", "Role" }
                        {[["all", "All"], ["Tank", "Tank"], ["Damage", "Damage"], ["Support", "Support"]].iter().map(|&[k, label]| {
                            let active = rf.eq_ignore_ascii_case(k);
                            rsx! {
                                button {
                                    key: "{k}",
                                    class: if active { "filter-chip active" } else { "filter-chip" },
                                    onclick: move |_| role_filter.set(k),
                                    "{label}"
                                }
                            }
                        })}
                    }
                    p { class: "filter-hint", "Filters apply to loaded rows (this page)." }
                }

                if rows.is_empty() {
                    p { class: "empty-state", "No matches match these filters on this page." }
                } else {
                    // W5b: fixed columns for E/D/A | dmg | heal so values
                    // align across rows (tabular-nums, right-aligned). Header
                    // uses the same grid track template as each card.
                    div { class: "match-list",
                        div { class: "match-list-head", aria_hidden: "true",
                            span { class: "match-h-outcome" }
                            span { class: "match-h-id" }
                            div { class: "match-stats",
                                span { class: "match-h-stat", "E" }
                                span { class: "match-h-stat", "D" }
                                span { class: "match-h-stat", "A" }
                                span { class: "match-h-stat match-h-wide", "DMG" }
                                span { class: "match-h-stat match-h-wide", "HEAL" }
                            }
                            span { class: "match-h-date" }
                        }
                        div { class: "match-cards",
                            for m in rows.iter() {
                                {
                                    let oc = outcome_class(&m.outcome);
                                    let date = format_date(&m.played_at);
                                    let abs = m.played_at.format("%Y-%m-%d %H:%M UTC").to_string();
                                    let map_label = if m.map_name.trim().is_empty() {
                                        "Unknown map".to_string()
                                    } else {
                                        m.map_name.clone()
                                    };
                                    let role_label = stored_role_label(&m.role);
                                    let fields = &m.suspect_fields;
                                    let map_unsure = cell_is_unsure(fields, "map");
                                    let mode_unsure = cell_is_unsure(fields, "mode");
                                    let result_unsure = cell_is_unsure(fields, "result");
                                    let hero_unsure = cell_is_unsure(fields, "hero");
                                    let mit_unsure = cell_is_unsure(fields, "mit");
                                    let new_reader = show_new_reader_badge(&m.recognizer);
                                    let mode_text = m.game_mode.clone();
                                    let mit_text = format!("MIT {}", m.mitigation);
                                    rsx! {
                                        div { class: "match-card", key: "{m.id}",
                                            {marked_value(
                                                &format!("match-outcome {oc}"),
                                                &m.outcome,
                                                None,
                                                result_unsure,
                                            )}
                                            div { class: "match-identity",
                                                div { class: "match-hero-line",
                                                    {marked_value("match-hero", &m.hero, None, hero_unsure)}
                                                    if new_reader {
                                                        span {
                                                            class: "tracker-badge",
                                                            title: NEW_READER_TITLE,
                                                            "{NEW_READER_BADGE}"
                                                        }
                                                    }
                                                }
                                                div { class: "match-map",
                                                    {marked_value("", &map_label, None, map_unsure)}
                                                    if mode_unsure {
                                                        span {
                                                            " · "
                                                            {marked_value("", &mode_text, None, true)}
                                                        }
                                                    }
                                                    " · "
                                                    "{role_label}"
                                                    if mit_unsure {
                                                        span {
                                                            " · "
                                                            {marked_value("", &mit_text, None, true)}
                                                        }
                                                    }
                                                }
                                            }
                                            div { class: "match-stats",
                                                {marked_value(
                                                    "match-stat",
                                                    &m.elims.to_string(),
                                                    Some("Eliminations"),
                                                    cell_is_unsure(fields, "e"),
                                                )}
                                                {marked_value(
                                                    "match-stat",
                                                    &m.deaths.to_string(),
                                                    Some("Deaths"),
                                                    cell_is_unsure(fields, "d"),
                                                )}
                                                {marked_value(
                                                    "match-stat",
                                                    &m.assists.to_string(),
                                                    Some("Assists"),
                                                    cell_is_unsure(fields, "a"),
                                                )}
                                                {marked_value(
                                                    "match-stat match-stat-wide",
                                                    &m.damage.to_string(),
                                                    Some("Damage"),
                                                    cell_is_unsure(fields, "dmg"),
                                                )}
                                                {marked_value(
                                                    "match-stat match-stat-wide",
                                                    &m.healing.to_string(),
                                                    Some("Healing"),
                                                    cell_is_unsure(fields, "h"),
                                                )}
                                            }
                                            div { class: "match-date", title: "{abs}", "{date}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "stats-pagination",
                    button {
                        disabled: !can_prev,
                        onclick: move |_| {
                            let mut hist = cursor_history();
                            if hist.len() > 1 {
                                hist.pop();
                                let prev = hist.last().cloned().flatten();
                                cursor_history.set(hist);
                                page_cursor.set(prev);
                            }
                        },
                        "Previous"
                    }
                    button {
                        disabled: !has_next,
                        onclick: move |_| {
                            if let Some(nc) = &next_c {
                                let mut hist = cursor_history();
                                hist.push(Some(nc.clone()));
                                cursor_history.set(hist);
                                page_cursor.set(Some(nc.clone()));
                            }
                        },
                        "Next"
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::ApiResource;
    use chrono::{DateTime, TimeZone, Utc};

    fn played_at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 15, 12, 0, 0).unwrap()
    }

    fn row(
        id: &str,
        recognizer: &str,
        suspect: &[&str],
        hero: &str,
        map_name: &str,
        game_mode: &str,
        mitigation: u32,
    ) -> super::super::PersonalMatch {
        super::super::PersonalMatch {
            id: id.to_string(),
            hero: hero.to_string(),
            map_name: map_name.to_string(),
            game_mode: game_mode.to_string(),
            role: "Support".to_string(),
            outcome: "victory".to_string(),
            elims: 11,
            deaths: 22,
            assists: 33,
            damage: 4444,
            healing: 5555,
            mitigation,
            played_at: played_at(),
            recognizer: recognizer.to_string(),
            suspect_fields: suspect.iter().map(|name| (*name).to_string()).collect(),
        }
    }

    #[test]
    fn badge_only_for_a_recognizer_other_than_ocr_v1() {
        assert!(!show_new_reader_badge("ocr-v1"));
        assert!(!show_new_reader_badge(scuffed_types::RECOGNIZER_OCR_V1));
        assert!(show_new_reader_badge("cv-v1"));
        assert!(show_new_reader_badge("OCR-V1"));
        assert!(show_new_reader_badge(""));
    }

    #[test]
    fn unsure_marks_known_cells_and_ignores_unknown_names() {
        let fields = vec![
            "map".to_string(),
            "nope".to_string(),
            "e".to_string(),
            "r3.dmg".to_string(),
        ];
        assert!(cell_is_unsure(&fields, "map"));
        assert!(cell_is_unsure(&fields, "e"));
        assert!(!cell_is_unsure(&fields, "nope"));
        assert!(!cell_is_unsure(&fields, "r3.dmg"));
        assert!(!cell_is_unsure(&fields, "hero"));
        assert!(!cell_is_unsure(&[], "map"));
        for name in scuffed_types::SUSPECT_FIELD_NAMES {
            let only = vec![(*name).to_string()];
            assert!(cell_is_unsure(&only, name), "{name}");
            assert!(!cell_is_unsure(&only, "not-a-cell"));
        }
    }

    fn resource_of<T: Clone + 'static>(value: Option<T>) -> ApiResource<T> {
        let refresh = use_signal(|| 0u64);
        let error = use_signal(|| None::<String>);
        let truncated = use_signal(|| false);
        let shown = use_signal(|| 0usize);
        let page_budget = use_signal(|| 1usize);
        let data = use_resource(move || {
            let value = value.clone();
            async move { value }
        });
        ApiResource {
            data,
            refresh,
            error,
            truncated,
            shown,
            page_budget,
        }
    }

    fn pump(dom: &mut VirtualDom) {
        for _ in 0..8 {
            dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        }
    }

    #[test]
    fn history_rows_show_the_badge_and_unsure_marks() {
        fn view() -> Element {
            let page = super::super::MatchPage {
                data: vec![
                    row(
                        "new",
                        "cv-v1",
                        &[
                            "map", "mode", "result", "hero", "e", "a", "d", "dmg", "h", "mit",
                            "nope",
                        ],
                        "Ana",
                        "Ilios",
                        "push",
                        6666,
                    ),
                    row("old", "ocr-v1", &[], "Reinhardt", "Havana", "control", 6),
                ],
                next_cursor: None,
            };
            let matches = resource_of(Some(page));
            let page_cursor = use_signal(|| None::<String>);
            let cursor_history = use_signal(|| vec![None::<String>]);
            let outcome = use_signal(|| "all");
            let role = use_signal(|| "all");
            history_tab(matches, page_cursor, cursor_history, outcome, role)
        }

        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        pump(&mut dom);
        let html = dioxus_ssr::render(&dom);

        assert_eq!(html.matches(NEW_READER_BADGE).count(), 1, "{html}");
        assert!(
            html.contains("class=\"tracker-badge\""),
            "badge uses the existing tracker badge style: {html}"
        );
        assert!(
            html.contains(&format!("title=\"{NEW_READER_TITLE}\"")),
            "{html}"
        );
        assert!(html.contains("Reinhardt"), "{html}");
        assert!(
            !html.contains("control"),
            "an unflagged mode stays off the row: {html}"
        );
        assert!(html.contains("push"), "{html}");
        assert!(html.contains("6666"), "{html}");
        assert!(
            !html.contains(">6<") && !html.contains(">MIT 6<"),
            "an unflagged mitigation value stays off the row: {html}"
        );
        assert_eq!(
            html.matches("stat-unsure-mark").count(),
            scuffed_types::SUSPECT_FIELD_NAMES.len(),
            "unknown names add no mark: {html}"
        );
        assert_eq!(
            html.matches(&format!("aria-label=\"{UNSURE_TEXT}\""))
                .count(),
            scuffed_types::SUSPECT_FIELD_NAMES.len(),
            "{html}"
        );
        assert!(
            html.contains(&format!(
                "class=\"stat-unsure\" title=\"{UNSURE_TEXT}\">Ilios"
            )),
            "{html}"
        );
        assert!(html.contains("class=\"match-stat stat-unsure\""), "{html}");
        assert!(html.contains(">11<") || html.contains(">11"), "{html}");
    }
}
