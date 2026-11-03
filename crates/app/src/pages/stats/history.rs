use dioxus::prelude::*;

use crate::components::ui::ChipButton;
use crate::hooks::ApiResource;

use super::game::{match_path, open_match};
use super::history_filter::{
    ALL, HISTORY_CSS, UNKNOWN_MAP, filter_rows, map_rows, map_table, mode_label, page_modes,
};
use super::role::game_role_label;
use super::{MatchPage, format_date, load_error_state};

/// The tracker stores outcomes as `victory` / `defeat` / `draw` (see the
/// upload filter in site-server routes/stats.rs); `win` / `loss` are accepted
/// as aliases. DEFEAT must land on the serious/danger class; before this
/// matched only `win`/`loss`, every card fell through to the draw class and
/// DEFEAT rendered gold.
pub(super) fn outcome_class(outcome: &str) -> &'static str {
    match outcome.to_lowercase().as_str() {
        "victory" | "win" => "outcome-win",
        "defeat" | "loss" => "outcome-loss",
        _ => "outcome-draw",
    }
}

/// Visible label when a row was not read by the old Tesseract reader.
const NEW_READER_BADGE: &str = "New reader";
/// Short label when the row is too narrow for the full badge.
const NEW_READER_BADGE_SHORT: &str = "New";
/// Tooltip for [`NEW_READER_BADGE`].
const NEW_READER_TITLE: &str = "Read by the new stat reader";
/// Shared tail of the unsure-cell tooltip and of the tap-to-show note. The
/// stat's display name is the prefix.
const UNSURE_DETAIL: &str = "might be misread. Check it in the tracker.";
/// Tail of the screen reader text inside the mark button. The stat's display
/// name is the prefix, so every mark in a row has its own accessible name.
const UNSURE_SR_DETAIL: &str = "might be misread";

/// Id of the note a mark controls. Built only from the match id and the stat
/// name, with anything that is not id-safe turned into an underscore. Ids are
/// deterministic (no counters or random values), so server rendering and
/// hydration always agree, and two marks never share an id on one page.
fn unsure_note_id(match_id: &str, stat_name: &str) -> String {
    // Mapping to _ could collide (a/b vs a_b), but real match ids are record ids that cannot.
    let safe = |text: &str| -> String {
        text.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect()
    };
    format!("unsure-note-{}-{}", safe(match_id), safe(stat_name))
}

fn unsure_title(stat_name: &str) -> String {
    format!("{stat_name}: {UNSURE_DETAIL}")
}

/// Screen reader text inside the mark button, for example "Damage might be
/// misread".
fn unsure_sr_text(stat_name: &str) -> String {
    format!("{stat_name} {UNSURE_SR_DETAIL}")
}

/// `true` when this row should show the new-reader badge.
/// Empty and missing ids count as `ocr-v1`.
pub(super) fn show_new_reader_badge(recognizer: &str) -> bool {
    scuffed_types::effective_recognizer(recognizer) != scuffed_types::RECOGNIZER_OCR_V1
}

/// `true` when `name` is a known match cell and this row flagged it.
/// Names outside that set are ignored.
pub(super) fn cell_is_unsure(suspect_fields: &[String], name: &str) -> bool {
    scuffed_types::SUSPECT_FIELD_NAMES.contains(&name)
        && suspect_fields.iter().any(|field| field == name)
}

/// Question mark button that stays outside the clipped value and is never
/// inside a link (History rows use a sibling overlay link), and neither is its
/// note, so tapping the note does not open the game. Tapping the mark,
/// clicking it, or pressing Enter or Space on it shows [`unsure_title`] as a
/// small note (a polite status region that is always present and empty while
/// closed, wired with aria-controls) next to the value, so phones and keyboards get the same words as
/// the hover tooltip. "{stat} might be misread" is real text inside the button
/// for screen readers (which skip aria-label on a plain span), so each mark
/// has its own accessible name. Its hit box is a 24 by 24 pixel `::after` box,
/// so a tap a little off the small glyph still lands on the mark. Display only: it
/// changes no value and no average.
#[component]
fn UnsureMark(match_id: String, stat_name: String, start_open: bool) -> Element {
    let mut local_open = use_signal(move || start_open);
    // Inside a History row the open state lives on the row, so only one note
    // per row is open. Elsewhere (the game page) each mark keeps its own.
    let row = use_hook(try_consume_context::<RowNotes>);
    let note_id = unsure_note_id(&match_id, &stat_name);
    let is_open = match row {
        Some(RowNotes(shared)) => shared.read().as_deref() == Some(stat_name.as_str()),
        None => local_open(),
    };
    // The note is always in the page and empty while closed, so screen readers
    // register the live region before its text changes and announce it.
    let (note_class, note) = if is_open {
        (
            "stat-unsure-note stat-unsure-note-open",
            unsure_title(&stat_name),
        )
    } else {
        ("stat-unsure-note", String::new())
    };
    let toggled_name = stat_name.clone();
    let sr_text = unsure_sr_text(&stat_name);
    rsx! {
        button {
            class: "stat-unsure-mark",
            r#type: "button",
            aria_expanded: if is_open { "true" } else { "false" },
            aria_controls: "{note_id}",
            onclick: move |_| match row {
                Some(RowNotes(mut shared)) => {
                    let next = next_open_note(shared.peek().as_deref(), &toggled_name);
                    shared.set(next);
                }
                None => local_open.toggle(),
            },
            span { aria_hidden: "true", "?" }
            span { class: "sr-only", "{sr_text}" }
        }
        span { class: "{note_class}", id: "{note_id}", role: "status", "{note}" }
    }
}

/// The stat whose note is open in one History row, shared with every
/// [`UnsureMark`] in that row through context.
#[derive(Clone, Copy)]
struct RowNotes(Signal<Option<String>>);

/// Which note is open after `clicked` is toggled while `current` is open.
/// Clicking the open one closes it, clicking another closes the open one and
/// opens that one, so a row never has two notes open (they would overlap).
fn next_open_note(current: Option<&str>, clicked: &str) -> Option<String> {
    if current == Some(clicked) {
        None
    } else {
        Some(clicked.to_string())
    }
}

/// Wraps one History row so its marks share the open-note state. It adds no
/// element of its own, so the row's markup and layout are unchanged.
#[component]
fn RowNotesScope(#[props(default)] open: Option<String>, children: Element) -> Element {
    let shared = use_signal(move || open.clone());
    use_context_provider(move || RowNotes(shared));
    children
}

/// One match-row value. An unsure cell gets the [`UnsureMark`] button, its
/// hidden "{stat} might be misread" text, and [`unsure_title`]. A sure numeric cell
/// keeps the stat name as its title.
pub(super) fn marked_value(
    match_id: &str,
    class: &str,
    text: &str,
    stat_name: &str,
    titled_when_sure: bool,
    unsure: bool,
) -> Element {
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
    let title = if unsure {
        Some(unsure_title(stat_name))
    } else if titled_when_sure {
        Some(stat_name.to_string())
    } else {
        None
    };
    if let Some(title) = title {
        if unsure {
            rsx! {
                span {
                    class: "{class}",
                    title: "{title}",
                    span { class: "stat-value-text", "{text}" }
                    UnsureMark {
                        match_id: match_id.to_string(),
                        stat_name: stat_name.to_string(),
                        start_open: false,
                    }
                }
            }
        } else if class.is_empty() {
            rsx! {
                span {
                    title: "{title}",
                    span { class: "stat-value-text", "{text}" }
                }
            }
        } else {
            rsx! {
                span {
                    class: "{class}",
                    title: "{title}",
                    span { class: "stat-value-text", "{text}" }
                }
            }
        }
    } else if class.is_empty() {
        rsx! { span { class: "stat-value-text", "{text}" } }
    } else {
        rsx! {
            span {
                class: "{class}",
                span { class: "stat-value-text", "{text}" }
            }
        }
    }
}

pub(super) fn history_tab(
    matches: ApiResource<MatchPage>,
    mut page_cursor: Signal<Option<String>>,
    mut cursor_history: Signal<Vec<Option<String>>>,
    mut outcome_filter: Signal<&'static str>,
    mut role_filter: Signal<&'static str>,
    mut mode_filter: Signal<String>,
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

            let mf = mode_filter();

            let rows = filter_rows(&page.data, of, rf, &mf);
            let modes = page_modes(&page.data, &mf);
            // Map results ignore the outcome chip: filtering to wins would
            // make every map read 100%.
            let maps = map_rows(filter_rows(&page.data, ALL, rf, &mf));

            rsx! {
                crate::styles::StyleOnce { id: "stats-history-p2", css: HISTORY_CSS }
                div { class: "stats-filters",
                    div { class: "filter-group", role: "group", aria_labelledby: "history-filter-outcome",
                        span { class: "filter-label", id: "history-filter-outcome", "Outcome" }
                        {[["all", "All"], ["win", "Win"], ["loss", "Loss"], ["draw", "Draw"]].iter().map(|&[k, label]| {
                            rsx! {
                                ChipButton {
                                    key: "{k}",
                                    class: "filter-chip".to_string(),
                                    label: label.to_string(),
                                    pressed: of == k,
                                    onclick: move |_| outcome_filter.set(k),
                                }
                            }
                        })}
                    }
                    div { class: "filter-group", role: "group", aria_labelledby: "history-filter-role",
                        span { class: "filter-label", id: "history-filter-role", "Role" }
                        {[["all", "All"], ["Tank", "Tank"], ["Damage", "Damage"], ["Support", "Support"]].iter().map(|&[k, label]| {
                            rsx! {
                                ChipButton {
                                    key: "{k}",
                                    class: "filter-chip".to_string(),
                                    label: label.to_string(),
                                    pressed: rf.eq_ignore_ascii_case(k),
                                    onclick: move |_| role_filter.set(k),
                                }
                            }
                        })}
                    }
                    div { class: "filter-group", role: "group", aria_labelledby: "history-filter-mode",
                        span { class: "filter-label", id: "history-filter-mode", "Mode" }
                        ChipButton {
                            key: "{ALL}",
                            class: "filter-chip".to_string(),
                            label: "All".to_string(),
                            pressed: mf == ALL,
                            onclick: move |_| mode_filter.set(ALL.to_string()),
                        }
                        for mode in modes {
                            {
                                let active = mf.eq_ignore_ascii_case(&mode);
                                let pick = mode.clone();
                                rsx! {
                                    ChipButton {
                                        key: "{mode}",
                                        class: "filter-chip".to_string(),
                                        label: mode.clone(),
                                        pressed: active,
                                        onclick: move |_| mode_filter.set(pick.clone()),
                                    }
                                }
                            }
                        }
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
                                        UNKNOWN_MAP.to_string()
                                    } else {
                                        m.map_name.clone()
                                    };
                                    let role_label = game_role_label(&m.role, &m.hero);
                                    let fields = m.suspect_fields.as_deref().unwrap_or(&[]);
                                    let map_unsure = cell_is_unsure(fields, "map");
                                    let mode_unsure = cell_is_unsure(fields, "mode");
                                    let result_unsure = cell_is_unsure(fields, "result");
                                    let hero_unsure = cell_is_unsure(fields, "hero");
                                    let mit_unsure = cell_is_unsure(fields, "mit");
                                    let new_reader =
                                        show_new_reader_badge(m.recognizer.as_deref().unwrap_or(""));
                                    let mode_text = mode_label(&m.game_mode, &m.map_name);
                                    let mit_text = format!("MIT {}", m.mitigation);
                                    let href = match_path(&m.id);
                                    let target_id = m.id.clone();
                                    rsx! {
                                        // Stretched link: the anchor is an empty overlay that
                                        // covers the whole row, and the marked values sit
                                        // above it, so no button is ever inside the link.
                                        RowNotesScope { key: "{m.id}",
                                            div {
                                                class: "match-card match-card-link match-row-compact",
                                                a {
                                                    class: "match-card-overlay",
                                                    href: "{href}",
                                                    title: "{abs}",
                                                    onclick: move |evt: MouseEvent| open_match(evt, target_id.clone()),
                                                    span { class: "sr-only", "View game: {m.outcome} on {map_label}, {date}" }
                                                }
                                                {marked_value(
                                                    &m.id,
                                                    &format!("match-outcome {oc}"),
                                                    &m.outcome,
                                                    "Result",
                                                    false,
                                                    result_unsure,
                                                )}
                                                div { class: "match-identity match-identity-compact",
                                                    div { class: "match-hero-line",
                                                        {marked_value(&m.id, "match-hero", &m.hero, "Hero", false, hero_unsure)}
                                                        if new_reader {
                                                            span {
                                                                class: "tracker-badge reader-badge",
                                                                title: NEW_READER_TITLE,
                                                                span { class: "reader-badge-full", "{NEW_READER_BADGE}" }
                                                                span { class: "reader-badge-short", "{NEW_READER_BADGE_SHORT}" }
                                                            }
                                                        }
                                                    }
                                                    div { class: "match-map",
                                                        {marked_value(&m.id, "match-map-name", &map_label, "Map", false, map_unsure)}
                                                        span { class: "match-map-sep", " · " }
                                                        {marked_value(&m.id, "match-mode", &mode_text, "Mode", false, mode_unsure)}
                                                        span { class: "match-map-sep", " · " }
                                                        "{role_label}"
                                                        if mit_unsure {
                                                            span { class: "match-map-sep", " · " }
                                                            {marked_value(&m.id, "stat-flagged", &mit_text, "Mitigation", false, true)}
                                                        }
                                                    }
                                                }
                                                div { class: "match-stats",
                                                    {marked_value(
                                                        &m.id,
                                                        "match-stat",
                                                        &m.elims.to_string(),
                                                        "Eliminations",
                                                        true,
                                                        cell_is_unsure(fields, "e"),
                                                    )}
                                                    {marked_value(
                                                        &m.id,
                                                        "match-stat",
                                                        &m.deaths.to_string(),
                                                        "Deaths",
                                                        true,
                                                        cell_is_unsure(fields, "d"),
                                                    )}
                                                    {marked_value(
                                                        &m.id,
                                                        "match-stat",
                                                        &m.assists.to_string(),
                                                        "Assists",
                                                        true,
                                                        cell_is_unsure(fields, "a"),
                                                    )}
                                                    {marked_value(
                                                        &m.id,
                                                        "match-stat match-stat-wide",
                                                        &m.damage.to_string(),
                                                        "Damage",
                                                        true,
                                                        cell_is_unsure(fields, "dmg"),
                                                    )}
                                                    {marked_value(
                                                        &m.id,
                                                        "match-stat match-stat-wide",
                                                        &m.healing.to_string(),
                                                        "Healing",
                                                        true,
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
                {map_table(&maps)}
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
            recognizer: Some(recognizer.to_string()),
            suspect_fields: Some(suspect.iter().map(|name| (*name).to_string()).collect()),
        }
    }

    #[test]
    fn badge_only_for_a_recognizer_other_than_ocr_v1() {
        assert!(!show_new_reader_badge("ocr-v1"));
        assert!(!show_new_reader_badge(scuffed_types::RECOGNIZER_OCR_V1));
        assert!(show_new_reader_badge("cv-v1"));
        assert!(show_new_reader_badge("OCR-V1"));
        assert!(!show_new_reader_badge(""));
        assert!(!show_new_reader_badge(scuffed_types::effective_recognizer(
            ""
        )));
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

    /// Fails if any `<button` appears between an `<a ` and its `</a>`.
    fn assert_no_button_inside_anchor(html: &str) {
        let mut rest = html;
        let mut anchors = 0;
        while let Some(start) = rest.find("<a ") {
            let after = &rest[start..];
            let end = after.find("</a>").expect("anchor is closed");
            let anchor = &after[..end];
            assert!(
                !anchor.contains("<button"),
                "a button inside a link is invalid HTML: {anchor}"
            );
            assert!(
                !anchor[3..].contains("<a "),
                "anchors do not nest: {anchor}"
            );
            anchors += 1;
            rest = &after[end + 4..];
        }
        assert!(anchors > 0, "the page has links to check: {html}");
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
                    row("blank", "", &[], "Brigitte", "Nepal", "flashpoint", 8),
                ],
                next_cursor: None,
            };
            let matches = resource_of(Some(page));
            let page_cursor = use_signal(|| None::<String>);
            let cursor_history = use_signal(|| vec![None::<String>]);
            let outcome = use_signal(|| "all");
            let role = use_signal(|| "all");
            let mode = use_signal(|| "all".to_string());
            history_tab(matches, page_cursor, cursor_history, outcome, role, mode)
        }

        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        pump(&mut dom);
        let html = dioxus_ssr::render(&dom);

        assert_eq!(html.matches(NEW_READER_BADGE).count(), 1, "{html}");
        for id in ["new", "old", "blank"] {
            assert!(
                html.contains(&format!("href=\"/stats/match/{id}\"")),
                "every row links to its game page: {html}"
            );
        }
        assert!(
            html.contains("class=\"tracker-badge reader-badge\""),
            "badge uses the existing tracker badge style: {html}"
        );
        assert!(html.contains(NEW_READER_BADGE_SHORT), "{html}");
        assert_eq!(NEW_READER_BADGE, "New reader");
        assert!(
            !html.to_lowercase().contains("alpha"),
            "the new reader is no longer labelled alpha: {html}"
        );
        assert!(!NEW_READER_TITLE.contains("testing"), "{NEW_READER_TITLE}");
        assert!(
            html.contains(&format!("title=\"{NEW_READER_TITLE}\"")),
            "{html}"
        );
        assert!(
            !html.contains("aria-label="),
            "unsure text is real text inside the button, not an aria-label: {html}"
        );
        assert!(html.contains("Reinhardt"), "{html}");
        assert!(
            html.contains("class=\"match-mode\"><span class=\"stat-value-text\">Control<"),
            "a compact row always shows its mode, written the usual way: {html}"
        );
        assert!(
            html.contains("class=\"match-mode\"><span class=\"stat-value-text\">Flashpoint<"),
            "an empty recognizer does not flag the row: {html}"
        );
        assert!(
            html.contains("class=\"match-mode stat-unsure\""),
            "a flagged mode keeps its mark: {html}"
        );
        assert!(html.contains("Push"), "{html}");
        assert!(
            html.contains("match-identity match-identity-compact"),
            "{html}"
        );
        for chip in ["Push", "Control", "Flashpoint"] {
            assert!(
                html.contains(&format!("class=\"filter-chip\">{chip}<")),
                "mode chip for {chip}: {html}"
            );
        }
        assert!(html.contains("class=\"history-maps\""), "{html}");
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
        assert_eq!(UNSURE_SR_DETAIL, "might be misread");
        assert!(!html.contains("(unsure)"), "{html}");
        assert!(!html.contains("(might be misread)"), "{html}");
        assert!(html.contains("class=\"sr-only\""), "{html}");
        let titles = [
            "Result",
            "Hero",
            "Map",
            "Mode",
            "Eliminations",
            "Assists",
            "Deaths",
            "Damage",
            "Healing",
            "Mitigation",
        ];
        assert_eq!(titles.len(), scuffed_types::SUSPECT_FIELD_NAMES.len());
        for name in titles {
            let sr = format!("<span class=\"sr-only\">{name} might be misread</span>");
            assert_eq!(
                html.matches(&sr).count(),
                1,
                "each mark names its stat for screen readers: {html}"
            );
            let title = unsure_title(name);
            assert_eq!(
                html.matches(&format!("title=\"{title}\"")).count(),
                1,
                "{name} tooltip missing: {html}"
            );
        }
        assert!(
            html.contains("Deaths: might be misread. Check it in the tracker."),
            "{html}"
        );
        assert!(!UNSURE_DETAIL.contains('\u{2014}'), "{UNSURE_DETAIL}");
        assert!(
            !html.contains("title=\"Might be misread"),
            "the tooltip must name the stat: {html}"
        );
        assert!(
            !html.contains("the reader was unsure") && !html.contains("unsure read"),
            "old unsure copy must stay off the row: {html}"
        );
        assert!(
            html.contains("class=\"stat-value-text\">Ana</span><button class=\"stat-unsure-mark\""),
            "the mark sits outside the clipped hero text: {html}"
        );
        assert!(
            html.contains("class=\"stat-flagged stat-unsure\""),
            "flagged mode and mitigation keep their own room: {html}"
        );
        assert!(html.contains("class=\"match-stat stat-unsure\""), "{html}");
        assert!(html.contains(">11<") || html.contains(">11"), "{html}");
        let marks = scuffed_types::SUSPECT_FIELD_NAMES.len();
        let open_tags: Vec<&str> = html
            .match_indices("<button class=\"stat-unsure-mark\"")
            .map(|(at, _)| &html[at..at + html[at..].find('>').unwrap()])
            .collect();
        assert_eq!(
            open_tags.len(),
            marks,
            "every mark is a real button, so it takes keyboard focus: {html}"
        );
        for tag in open_tags {
            assert!(tag.contains("type=\"button\""), "{tag}");
            assert!(
                tag.contains("aria-expanded=\"false\""),
                "every mark starts collapsed: {tag}"
            );
            assert!(
                !tag.contains("tabindex"),
                "a button needs no tabindex: {tag}"
            );
        }
        assert_no_button_inside_anchor(&html);
        let mut controls: Vec<&str> = html
            .split("aria-controls=\"")
            .skip(1)
            .map(|rest| rest.split('"').next().unwrap())
            .collect();
        assert_eq!(controls.len(), marks, "{html}");
        controls.sort_unstable();
        controls.dedup();
        assert_eq!(
            controls.len(),
            marks,
            "every mark controls its own note: {html}"
        );
        let mut expected: Vec<String> = titles
            .iter()
            .map(|name| format!("unsure-note-new-{}", name.to_lowercase()))
            .collect();
        expected.sort_unstable();
        assert_eq!(
            controls,
            expected.iter().map(String::as_str).collect::<Vec<_>>(),
            "ids come from the match id and the stat name: {html}"
        );
        for id in controls {
            let (tag, body) = note_by_id(&html, id);
            assert!(tag.starts_with("<span "), "{tag}");
            assert!(tag.contains("class=\"stat-unsure-note\""), "{tag}");
            assert!(tag.contains("role=\"status\""), "{tag}");
            assert!(
                !tag.contains("stat-unsure-note-open"),
                "closed note is collapsed: {tag}"
            );
            assert_eq!(body, "", "the live region is empty while closed: {html}");
        }
        assert_eq!(
            html.matches("class=\"match-card-overlay\"").count(),
            3,
            "one overlay link per row: {html}"
        );
        assert!(
            html.contains("View game: victory on Ilios,"),
            "the overlay link has a readable name: {html}"
        );
        let overlay_tags: Vec<&str> = html
            .match_indices("<a class=\"match-card-overlay\"")
            .map(|(at, _)| &html[at..at + html[at..].find('>').unwrap()])
            .collect();
        assert_eq!(overlay_tags.len(), 3, "{html}");
        for tag in overlay_tags {
            assert!(
                tag.contains("title=\"2026-03-15 12:00 UTC\""),
                "the overlay link carries the exact time as its hover title: {tag}"
            );
        }
    }

    /// The opening tag and the text of the span with this id.
    fn note_by_id<'a>(html: &'a str, id: &str) -> (&'a str, &'a str) {
        let at = html
            .find(&format!("id=\"{id}\""))
            .unwrap_or_else(|| panic!("note {id} is missing: {html}"));
        let start = html[..at].rfind('<').unwrap();
        let tag_end = start + html[start..].find('>').unwrap();
        let body_end = tag_end + html[tag_end..].find("</span>").unwrap();
        (&html[start..tag_end], &html[tag_end + 1..body_end])
    }

    #[test]
    fn open_note_is_a_status_region_that_the_button_controls() {
        fn view() -> Element {
            rsx! {
                div { class: "match-card match-card-link",
                    a { class: "match-card-overlay", href: "/stats/match/x",
                        span { class: "sr-only", "View game" }
                    }
                    UnsureMark {
                        match_id: "x".to_string(),
                        stat_name: "Deaths".to_string(),
                        start_open: true,
                    }
                    UnsureMark {
                        match_id: "x".to_string(),
                        stat_name: "Hero".to_string(),
                        start_open: false,
                    }
                }
            }
        }
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);

        assert_eq!(html.matches("aria-expanded=\"true\"").count(), 1, "{html}");
        assert_eq!(html.matches("aria-expanded=\"false\"").count(), 1, "{html}");
        assert!(
            html.contains("aria-controls=\"unsure-note-x-deaths\""),
            "{html}"
        );
        assert!(
            html.contains("aria-controls=\"unsure-note-x-hero\""),
            "{html}"
        );

        let (open_tag, open_body) = note_by_id(&html, "unsure-note-x-deaths");
        assert!(open_tag.contains("stat-unsure-note-open"), "{open_tag}");
        assert!(open_tag.contains("role=\"status\""), "{open_tag}");
        assert_eq!(
            open_body,
            "Deaths: might be misread. Check it in the tracker."
        );

        let (closed_tag, closed_body) = note_by_id(&html, "unsure-note-x-hero");
        assert!(
            !closed_tag.contains("stat-unsure-note-open"),
            "{closed_tag}"
        );
        assert!(closed_tag.contains("role=\"status\""), "{closed_tag}");
        assert_eq!(closed_body, "", "{html}");

        assert_no_button_inside_anchor(&html);
        assert!(
            !html
                .split("</a>")
                .next()
                .unwrap()
                .contains("stat-unsure-note"),
            "the note is outside the row link: {html}"
        );
    }

    #[test]
    fn opening_a_note_closes_the_other_one_in_the_row() {
        // Nothing open: the click opens it.
        assert_eq!(next_open_note(None, "Deaths").as_deref(), Some("Deaths"));
        // Same mark again: it closes.
        assert_eq!(next_open_note(Some("Deaths"), "Deaths"), None);
        // Another mark: the first closes, the new one is the only one open.
        assert_eq!(
            next_open_note(Some("Deaths"), "Healing").as_deref(),
            Some("Healing")
        );
        // A run of clicks never leaves more than one open (the state holds one).
        let mut open: Option<String> = None;
        for clicked in ["Deaths", "Healing", "Healing", "Damage", "Deaths", "Deaths"] {
            open = next_open_note(open.as_deref(), clicked);
            assert!(open.as_deref().is_none_or(|name| name == clicked));
        }
        assert_eq!(open, None);
    }

    #[test]
    fn marks_in_one_row_show_at_most_one_open_note() {
        fn view() -> Element {
            rsx! {
                RowNotesScope { open: Some("Healing".to_string()),
                    div { class: "match-card match-card-link",
                        a { class: "match-card-overlay", href: "/stats/match/x",
                            span { class: "sr-only", "View game" }
                        }
                        UnsureMark {
                            match_id: "x".to_string(),
                            stat_name: "Deaths".to_string(),
                            start_open: false,
                        }
                        UnsureMark {
                            match_id: "x".to_string(),
                            stat_name: "Healing".to_string(),
                            start_open: false,
                        }
                    }
                }
                RowNotesScope {
                    div { class: "match-card match-card-link",
                        UnsureMark {
                            match_id: "y".to_string(),
                            stat_name: "Deaths".to_string(),
                            start_open: false,
                        }
                    }
                }
            }
        }
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);

        // The scope adds no markup: three marks, three always-rendered notes.
        assert_eq!(
            html.matches("class=\"stat-unsure-mark\"").count(),
            3,
            "{html}"
        );
        assert_eq!(html.matches("role=\"status\"").count(), 3, "{html}");
        // Only the row that was told to open one has one open; the other row
        // keeps its own state.
        assert_eq!(html.matches("stat-unsure-note-open").count(), 1, "{html}");
        assert_eq!(html.matches("aria-expanded=\"true\"").count(), 1, "{html}");
        assert_eq!(html.matches("aria-expanded=\"false\"").count(), 2, "{html}");
        let (healing_tag, healing_body) = note_by_id(&html, "unsure-note-x-healing");
        assert!(
            healing_tag.contains("stat-unsure-note-open"),
            "{healing_tag}"
        );
        assert_eq!(
            healing_body,
            "Healing: might be misread. Check it in the tracker."
        );
        let (deaths_tag, deaths_body) = note_by_id(&html, "unsure-note-x-deaths");
        assert!(
            !deaths_tag.contains("stat-unsure-note-open"),
            "{deaths_tag}"
        );
        assert_eq!(deaths_body, "", "{html}");
        assert_no_button_inside_anchor(&html);
    }

    #[test]
    fn note_ids_are_deterministic_and_id_safe() {
        assert_eq!(
            unsure_note_id("Ab 1/2", "Damage"),
            "unsure-note-ab_1_2-damage"
        );
        assert_eq!(unsure_note_id("m-1", "Hero"), unsure_note_id("m-1", "Hero"));
        assert_ne!(unsure_note_id("m-1", "Hero"), unsure_note_id("m-2", "Hero"));
        assert_ne!(unsure_note_id("m-1", "Hero"), unsure_note_id("m-1", "Map"));
    }

    #[test]
    fn unsure_note_text_matches_the_tooltip_and_has_no_dashes() {
        assert_eq!(
            unsure_title("Deaths"),
            "Deaths: might be misread. Check it in the tracker."
        );
        assert_eq!(unsure_sr_text("Damage"), "Damage might be misread");
        for text in [UNSURE_DETAIL, UNSURE_SR_DETAIL] {
            assert!(
                !text.contains('\u{2014}') && !text.contains('\u{2013}'),
                "{text}"
            );
        }
    }
}
