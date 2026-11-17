use dioxus::prelude::*;

use chrono::{DateTime, Utc};

use crate::components::ui::inline_confirm::focus_element_by_id_soon;
use crate::components::ui::{ChipButton, ChipRow, EmptyCard, SkeletonBars, btn_class};
use crate::hooks::ApiResource;
use crate::pages::stats_member::{STATS_VIEWS_CSS, STATS_VIEWS_CSS_ID, stats_load_error};

use super::MatchPage;
use super::bans_picks::{BANS_PICKS_CARD_CSS, bans_picks_line};
use super::game::{match_path, open_match};
use super::history_filter::{
    ALL, HISTORY_CSS, Outcome, UNKNOWN_MAP, filter_rows, map_rows, map_table, mode_label,
    outcome_kind, page_modes,
};
use super::rank::{
    RANK_CSS, RankChart, RankWindow, chart_series, rank_badge_line, rank_series_with_outcomes,
};
use super::role::game_role_label;

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

/// The word a stored outcome reads as. A result is always this word on a
/// soft fill, never a colour alone.
pub(super) fn outcome_word(outcome: &str) -> &'static str {
    match outcome_kind(outcome) {
        Outcome::Win => "Win",
        Outcome::Loss => "Loss",
        Outcome::Draw => "Draw",
    }
}

/// Shared pill classes for a result badge. `stat-value-text` keeps the unsure
/// underline and the mark placement of every other marked value.
fn outcome_pill_class(outcome: &str) -> &'static str {
    match outcome_kind(outcome) {
        Outcome::Win => "stat-value-text ui-pill ui-pill--ok",
        Outcome::Loss => "stat-value-text ui-pill ui-pill--danger",
        Outcome::Draw => "stat-value-text ui-pill ui-pill--neutral",
    }
}

/// The date of a game as the frames write it: "12 Nov 2026", on the
/// viewer's own day like the rank chart and the match page.
pub(super) fn game_date(played_at: &DateTime<Utc>) -> String {
    super::local_day(*played_at).format("%-d %b %Y").to_string()
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
/// so a tap a little off the small glyph still lands on the mark. The open note
/// starts outside that box, and a click or tap on the note closes it too.
/// Display only: it changes no value and no average.
#[component]
fn UnsureMark(match_id: String, stat_name: String, start_open: bool) -> Element {
    let local_open = use_signal(move || start_open);
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
    let note_name = stat_name.clone();
    let sr_text = unsure_sr_text(&stat_name);
    rsx! {
        button {
            class: "stat-unsure-mark",
            r#type: "button",
            aria_expanded: if is_open { "true" } else { "false" },
            aria_controls: "{note_id}",
            onclick: move |_| toggle_note(row, local_open, &toggled_name),
            span { aria_hidden: "true", "?" }
            span { class: "visually-hidden", "{sr_text}" }
        }
        // A click or tap on the open note closes it, like the mark. The note is
        // never inside the row link, so the click opens nothing. Closed, it is
        // a clipped 1px region, so it takes no clicks. The mark stays the
        // keyboard control, so the note gets no role or tab stop for this.
        span {
            class: "{note_class}",
            id: "{note_id}",
            role: "status",
            onclick: move |_| {
                if is_open {
                    toggle_note(row, local_open, &note_name);
                }
            },
            "{note}"
        }
    }
}

/// Toggles the note of `stat_name`: through the row state inside a History row
/// (one open note per row), or through the mark's own state elsewhere. The mark
/// and its open note both call this.
fn toggle_note(row: Option<RowNotes>, mut local_open: Signal<bool>, stat_name: &str) {
    match row {
        Some(RowNotes(mut shared)) => {
            let next = next_open_note(shared.peek().as_deref(), stat_name);
            shared.set(next);
        }
        None => local_open.toggle(),
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
    marked_text(
        match_id,
        class,
        "stat-value-text",
        text,
        stat_name,
        titled_when_sure,
        unsure,
    )
}

/// The result of a game as a badge: the word Win, Loss or Draw on the shared
/// soft-fill pill, with the unsure mark when the reader flagged the result.
/// `class` is the cell class (`match-outcome`, `game-outcome`); the outcome
/// class is added to it.
pub(super) fn outcome_badge(match_id: &str, class: &str, outcome: &str, unsure: bool) -> Element {
    let oc = outcome_class(outcome);
    marked_text(
        match_id,
        &format!("{class} {oc}"),
        outcome_pill_class(outcome),
        outcome_word(outcome),
        "Result",
        false,
        unsure,
    )
}

/// [`marked_value`] with the class of the inner text span given.
fn marked_text(
    match_id: &str,
    class: &str,
    text_class: &str,
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
    let text_class = text_class.to_string();
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
                    span { class: "{text_class}", "{text}" }
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
                    span { class: "{text_class}", "{text}" }
                }
            }
        } else {
            rsx! {
                span {
                    class: "{class}",
                    title: "{title}",
                    span { class: "{text_class}", "{text}" }
                }
            }
        }
    } else if class.is_empty() {
        rsx! { span { class: "{text_class}", "{text}" } }
    } else {
        rsx! {
            span {
                class: "{class}",
                span { class: "{text_class}", "{text}" }
            }
        }
    }
}

/// What screen readers hear for the tab. Always rendered, only the text
/// changes.
fn history_status(loaded: Option<usize>, failed: bool, shown: usize) -> String {
    match loaded {
        // The failure card is an alert that says it; the status line stays
        // quiet so it is not announced twice.
        None if failed => String::new(),
        None => "Loading match history.".to_string(),
        Some(0) => "No games recorded yet.".to_string(),
        Some(_) if shown == 0 => "No games match these filters on this page.".to_string(),
        Some(1) if shown == 1 => "Showing 1 game.".to_string(),
        Some(total) => format!("Showing {shown} of {total} games on this page."),
    }
}

/// "No games yet", with the next step: connect the tracker.
#[component]
fn HistoryEmpty() -> Element {
    rsx! {
        EmptyCard {
            title: "No games recorded yet",
            message: "Your games show up here after the tracker uploads them.",
            actions: rsx! {
                // The install steps are on this page, as the summary card's
                // link says.
                a { class: btn_class(true), href: "{super::TRACKER_ANCHOR}", "Set up the tracker" }
            },
        }
    }
}

/// Open the page after the one shown. `next` comes from the page on screen,
/// so a second press before the new page lands names the same cursor: it is
/// not pushed again, or the trail would hold one page twice and Previous
/// would stay on it. Returns whether the trail moved.
fn push_next_page(trail: &mut Vec<Option<String>>, next: &str) -> bool {
    if trail.last().and_then(|c| c.as_deref()) == Some(next) {
        return false;
    }
    trail.push(Some(next.to_string()));
    true
}

/// Id of an Outcome filter chip, from its filter value.
fn outcome_chip_id(value: &str) -> String {
    format!("history-outcome-{value}")
}

/// Class of a Previous or Next step of the shared pager look.
fn step_class(off: bool) -> &'static str {
    if off {
        "pager-step is-disabled"
    } else {
        "pager-step"
    }
}

pub(super) fn history_tab(
    matches: ApiResource<MatchPage>,
    mut page_cursor: Signal<Option<String>>,
    mut cursor_history: Signal<Vec<Option<String>>>,
    mut outcome_filter: Signal<&'static str>,
    mut role_filter: Signal<&'static str>,
    mut mode_filter: Signal<String>,
    player: Option<String>,
) -> Element {
    let failed = matches.error.read().is_some();
    let data = matches.data.read();
    let page = data.as_ref().and_then(|d| d.as_ref());
    let of = outcome_filter();
    let rf = role_filter();
    let mf = mode_filter();
    let rows = page
        .map(|page| filter_rows(&page.data, of, rf, &mf))
        .unwrap_or_default();
    let status = history_status(page.map(|page| page.data.len()), failed, rows.len());
    // The cursor trail holds one entry per page opened, so its length is the
    // page number. The API returns no total, so there is no "of N".
    let page_no = cursor_history().len().max(1);
    let can_prev = page_no > 1;
    // An empty first page is "no games yet". An empty later page cannot
    // happen: a cursor is only returned when rows follow.
    let no_games = page.is_some_and(|page| page.data.is_empty());

    let body = match page {
        None if failed => stats_load_error(
            "match history",
            matches.error.read().clone(),
            matches.refresh,
        ),
        None => rsx! { SkeletonBars { rows: 5 } },
        Some(_) if no_games => rsx! { HistoryEmpty {} },
        Some(page) => {
            let has_next = page.next_cursor.is_some();
            let next_c = page.next_cursor.clone();
            let modes = page_modes(&page.data, &mf);
            // Map results ignore the outcome chip: filtering to wins would
            // make every map read 100%.
            let maps = map_rows(filter_rows(&page.data, ALL, rf, &mf));
            // Opt in: empty unless some loaded game has rank progress. The
            // outcome word lets the readout and table say Win or Loss.
            let rank_chart = chart_series(rank_series_with_outcomes(page.data.iter().map(|m| {
                (
                    m.id.as_str(),
                    m.played_at,
                    m.rank.as_ref(),
                    Some(m.outcome.as_str()),
                )
            })));

            rsx! {
                if !rank_chart.is_empty() {
                    RankChart { series: rank_chart, window: RankWindow::Page(page_no), player }
                }
                section { class: "ui-stack", aria_labelledby: "history-games-title",
                    h2 { id: "history-games-title", class: "ui-section-title", "Games" }
                    div { class: "stats-view-toolbar",
                        div { class: "stats-view-filter",
                            span { class: "stats-view-filter-label", aria_hidden: "true", "Outcome" }
                            ChipRow { label: "Outcome", scroll: true,
                                {[["all", "All"], ["win", "Win"], ["loss", "Loss"], ["draw", "Draw"]].iter().map(|&[k, label]| {
                                    rsx! {
                                        ChipButton {
                                            key: "{k}",
                                            id: outcome_chip_id(k),
                                            class: "chip".to_string(),
                                            label: label.to_string(),
                                            pressed: of == k,
                                            onclick: move |_| outcome_filter.set(k),
                                        }
                                    }
                                })}
                            }
                        }
                        div { class: "stats-view-filter",
                            span { class: "stats-view-filter-label", aria_hidden: "true", "Role" }
                            ChipRow { label: "Role", scroll: true,
                                {[["all", "All"], ["Tank", "Tank"], ["Damage", "Damage"], ["Support", "Support"]].iter().map(|&[k, label]| {
                                    rsx! {
                                        ChipButton {
                                            key: "{k}",
                                            class: "chip".to_string(),
                                            label: label.to_string(),
                                            pressed: rf.eq_ignore_ascii_case(k),
                                            onclick: move |_| role_filter.set(k),
                                        }
                                    }
                                })}
                            }
                        }
                        div { class: "stats-view-filter",
                            span { class: "stats-view-filter-label", aria_hidden: "true", "Mode" }
                            ChipRow { label: "Mode", scroll: true,
                                ChipButton {
                                    key: "{ALL}",
                                    class: "chip".to_string(),
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
                                                class: "chip".to_string(),
                                                label: mode.clone(),
                                                pressed: active,
                                                onclick: move |_| mode_filter.set(pick.clone()),
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    p { class: "stats-view-note", "Filters apply to the games loaded on this page." }

                    if rows.is_empty() {
                        div { class: "stats-view-card history-no-match",
                            p { class: "stats-view-note",
                                if has_next {
                                    "No games on this page match these filters. Clear them, or open the next page."
                                } else {
                                    "No games on this page match these filters. Clear them to see every game."
                                }
                            }
                            button {
                                r#type: "button",
                                id: "history-clear-filters",
                                class: "btn is-ghost",
                                onclick: move |_| {
                                    outcome_filter.set(ALL);
                                    role_filter.set(ALL);
                                    mode_filter.set(ALL.to_string());
                                    // This button leaves with the empty card:
                                    // focus the first filter it reset.
                                    focus_element_by_id_soon(outcome_chip_id(ALL));
                                },
                                "Clear filters"
                            }
                        }
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
                            ul { class: "match-cards",
                                for m in rows.iter() {
                                    {
                                        let date = game_date(&m.played_at);
                                        let abs = m.played_at.format("%Y-%m-%d %H:%M UTC").to_string();
                                        let machine_date = m.played_at.to_rfc3339();
                                        let result = outcome_word(&m.outcome);
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
                                                li {
                                                    class: "match-card match-card-link match-row-compact",
                                                    a {
                                                        class: "match-card-overlay",
                                                        href: "{href}",
                                                        title: "{abs}",
                                                        onclick: move |evt: MouseEvent| open_match(evt, target_id.clone()),
                                                        span { class: "visually-hidden", "View game: {result} on {map_label}, {date}" }
                                                    }
                                                    {outcome_badge(&m.id, "match-outcome", &m.outcome, result_unsure)}
                                                    div { class: "match-identity match-identity-compact",
                                                        div { class: "match-hero-line",
                                                            {marked_value(&m.id, "match-hero", &m.hero, "Hero", false, hero_unsure)}
                                                            if new_reader {
                                                                span {
                                                                    class: "ui-pill ui-pill--info reader-badge",
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
                                                    time { class: "match-date", datetime: "{machine_date}", title: "{abs}", "{date}" }
                                                    {rank_badge_line(&m.id, m.rank.as_ref())}
                                                    {bans_picks_line(&m.id, m.bans_picks.as_ref())}
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // The match list pages by cursor and the API returns no
                    // total, so this is Previous and Next in the shared pager
                    // look, with the page number as the status line. The ends
                    // are aria-disabled, so focus is not dropped.
                    div { class: "pager", id: "history-pager",
                        p { class: "pager-meta", id: "history-pager-meta", role: "status", "Page {page_no}" }
                        nav { class: "pager-pages", aria_label: "Match history pages",
                            button {
                                r#type: "button",
                                id: "history-pager-prev",
                                class: step_class(!can_prev),
                                aria_label: "Previous page",
                                aria_disabled: if !can_prev { "true" },
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
                                r#type: "button",
                                id: "history-pager-next",
                                class: step_class(!has_next),
                                aria_label: "Next page",
                                aria_disabled: if !has_next { "true" },
                                onclick: move |_| {
                                    if let Some(nc) = &next_c {
                                        let mut hist = cursor_history();
                                        if push_next_page(&mut hist, nc) {
                                            cursor_history.set(hist);
                                            page_cursor.set(Some(nc.clone()));
                                        }
                                    }
                                },
                                "Next"
                            }
                        }
                    }
                }
                {map_table(&maps)}
            }
        }
    };

    rsx! {
        crate::styles::StyleOnce { id: STATS_VIEWS_CSS_ID, css: STATS_VIEWS_CSS }
        crate::styles::StyleOnce { id: "stats-history-p2", css: HISTORY_CSS }
        crate::styles::StyleOnce { id: "stats-history-bans-picks", css: BANS_PICKS_CARD_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-stats-rank-css", css: RANK_CSS }
        div { class: "stats-view-body",
            p { class: "visually-hidden", role: "status", "{status}" }
            {body}
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
            bans_picks: None,
            rank: None,
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

    /// The tab with one loaded game and the Draw filter on, so no row
    /// matches. `next` is the page's next cursor.
    fn filtered_out(next: Option<&'static str>) -> String {
        thread_local! { static NEXT: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) }; }
        NEXT.with(|n| n.set(next));
        fn view() -> Element {
            let page = super::super::MatchPage {
                data: vec![row(
                    "old",
                    "ocr-v1",
                    &[],
                    "Reinhardt",
                    "Havana",
                    "control",
                    6,
                )],
                next_cursor: NEXT.with(|n| n.get()).map(str::to_string),
            };
            history_tab(
                resource_of(Some(page)),
                use_signal(|| None::<String>),
                use_signal(|| vec![None::<String>]),
                use_signal(|| "draw"),
                use_signal(|| "all"),
                use_signal(|| "all".to_string()),
                None,
            )
        }
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        pump(&mut dom);
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn no_match_copy_offers_the_next_page_only_when_there_is_one() {
        let last = filtered_out(None);
        assert!(last.contains("history-no-match"), "{last}");
        assert!(!last.contains("open the next page"), "{last}");
        assert!(last.contains("Clear them to see every game."), "{last}");
        let more = filtered_out(Some("c2"));
        assert!(
            more.contains("Clear them, or open the next page."),
            "{more}"
        );
        // Clear filters removes itself; focus goes to the Outcome All chip.
        assert!(
            last.contains(&format!("id=\"{}\"", outcome_chip_id(ALL))),
            "{last}"
        );
        let src = include_str!("history.rs");
        let body = &src[..src.find("#[cfg(test)]").unwrap()];
        let clear = body.find("\"Clear filters\"").unwrap();
        let handler = &body[body[..clear].rfind("onclick:").unwrap()..clear];
        assert!(
            handler.contains("focus_element_by_id_soon(outcome_chip_id(ALL))"),
            "{handler}"
        );
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
            history_tab(
                matches,
                page_cursor,
                cursor_history,
                outcome,
                role,
                mode,
                None,
            )
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
            html.contains("class=\"ui-pill ui-pill--info reader-badge\""),
            "badge uses the shared pill: {html}"
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
        // (Each mark button is checked for "no aria-label" with the other
        // mark tags below: the chip groups and the pager do carry one.)
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
                html.contains(&format!("class=\"chip\">{chip}<")),
                "mode chip for {chip}: {html}"
            );
        }
        for group in ["Outcome", "Role", "Mode"] {
            assert!(
                html.contains(&format!("role=\"group\" aria-label=\"{group}\"")),
                "the {group} chips are a named group: {html}"
            );
        }
        assert!(html.contains("class=\"ui-stack history-maps\""), "{html}");
        assert!(!html.contains("<h1") && !html.contains("<main"), "{html}");
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
        assert!(html.contains("class=\"visually-hidden\""), "{html}");
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
            let sr = format!("<span class=\"visually-hidden\">{name} might be misread</span>");
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
            assert!(
                !tag.contains("aria-label"),
                "unsure text is real text inside the button, not an aria-label: {tag}"
            );
        }
        // The always-rendered status line and the cursor pager.
        assert!(
            html.contains("<p class=\"visually-hidden\" role=\"status\">Showing 3 of 3 games on this page.</p>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<p class=\"pager-meta\" id=\"history-pager-meta\" role=\"status\">Page 1</p>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<nav class=\"pager-pages\" aria-label=\"Match history pages\">"),
            "{html}"
        );
        for step in ["prev", "next"] {
            let at = html
                .find(&format!("id=\"history-pager-{step}\""))
                .unwrap_or_else(|| panic!("{step}: {html}"));
            let tag = &html[at..at + html[at..].find('>').unwrap()];
            assert!(
                tag.contains("class=\"pager-step is-disabled\"")
                    && tag.contains("aria-disabled=\"true\"")
                    && !tag.contains(" disabled"),
                "one page: both ends are off but keep focus: {tag}"
            );
        }
        // Every row says its result as a word on the shared pill, and its
        // date as the frames write it.
        assert_eq!(
            html.matches("class=\"stat-value-text ui-pill ui-pill--ok\">Win<")
                .count(),
            3,
            "{html}"
        );
        assert!(
            html.contains("class=\"match-outcome outcome-win stat-unsure\""),
            "a flagged result keeps its mark: {html}"
        );
        assert_eq!(
            html.matches("<time class=\"match-date\" datetime=\"2026-03-15T12:00:00+00:00\" title=\"2026-03-15 12:00 UTC\">15 Mar 2026</time>").count(),
            3,
            "{html}"
        );
        assert_eq!(html.matches("<li class=\"match-card ").count(), 3, "{html}");
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
            html.contains("View game: Win on Ilios, 15 Mar 2026"),
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
                        span { class: "visually-hidden", "View game" }
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

    /// Opening state of the note with this id, read from the rendered page.
    fn is_open(html: &str, match_id: &str, stat: &str) -> bool {
        let id = unsure_note_id(match_id, stat);
        let (tag, body) = note_by_id(html, &id);
        let open = tag.contains("stat-unsure-note-open");
        assert_eq!(open, !body.is_empty(), "text only while open: {html}");
        open
    }

    #[test]
    fn a_click_on_the_open_note_closes_it() {
        use std::cell::Cell;
        thread_local! {
            static ROW: Cell<Option<RowNotes>> = const { Cell::new(None) };
        }
        /// Hands the row state of the surrounding scope to the test.
        #[component]
        fn Probe() -> Element {
            ROW.set(try_consume_context::<RowNotes>());
            rsx! {}
        }
        fn view() -> Element {
            rsx! {
                RowNotesScope { open: Some("Hero".to_string()),
                    div { class: "match-card match-card-link",
                        UnsureMark {
                            match_id: "x".to_string(),
                            stat_name: "Hero".to_string(),
                            start_open: false,
                        }
                        UnsureMark {
                            match_id: "x".to_string(),
                            stat_name: "Deaths".to_string(),
                            start_open: false,
                        }
                        Probe {}
                    }
                }
            }
        }
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        let row = ROW.get().expect("the marks share one row state");
        assert!(is_open(&dioxus_ssr::render(&dom), "x", "Hero"));

        // The mark and its open note call `toggle_note` with the same arguments,
        // so a click on the Hero note is this call. It closes the note.
        dom.in_scope(ScopeId::APP, || {
            toggle_note(Some(row), Signal::new(false), "Hero");
        });
        dom.render_immediate(&mut dioxus_core::NoOpMutations);
        let html = dioxus_ssr::render(&dom);
        assert!(!is_open(&html, "x", "Hero"), "{html}");
        assert!(!is_open(&html, "x", "Deaths"), "{html}");
        assert_eq!(html.matches("stat-unsure-note-open").count(), 0, "{html}");

        // Another mark opens its own note only, and a click on that note closes it.
        for expect_open in [true, false] {
            dom.in_scope(ScopeId::APP, || {
                toggle_note(Some(row), Signal::new(false), "Deaths");
            });
            dom.render_immediate(&mut dioxus_core::NoOpMutations);
            let html = dioxus_ssr::render(&dom);
            assert_eq!(is_open(&html, "x", "Deaths"), expect_open, "{html}");
            assert!(!is_open(&html, "x", "Hero"), "{html}");
        }
    }

    #[test]
    fn a_click_on_a_lone_open_note_closes_it() {
        // Outside a History row (the game page) the mark keeps its own state.
        let mut dom = VirtualDom::new(|| rsx! { div {} });
        dom.rebuild_in_place();
        dom.in_scope(ScopeId::APP, || {
            let own = Signal::new(true);
            toggle_note(None, own, "Hero");
            assert!(!own(), "a click on the open note closes it");
            toggle_note(None, own, "Hero");
            assert!(own(), "the mark opens it again");
        });
    }

    #[test]
    fn the_mark_and_its_note_both_take_clicks() {
        fn view() -> Element {
            rsx! {
                UnsureMark {
                    match_id: "x".to_string(),
                    stat_name: "Hero".to_string(),
                    start_open: true,
                }
            }
        }
        let mut dom = VirtualDom::new(view);
        let mut mutations = dioxus_core::Mutations::default();
        dom.rebuild(&mut mutations);
        let click_listeners = mutations
            .edits
            .iter()
            .filter(|edit| {
                matches!(
                    edit,
                    dioxus_core::Mutation::NewEventListener { name, .. } if *name == "click"
                )
            })
            .count();
        assert_eq!(click_listeners, 2, "one on the button, one on the note");
        // The note is still a status region, and neither it nor the mark is a link.
        let html = dioxus_ssr::render(&dom);
        let (tag, _) = note_by_id(&html, "unsure-note-x-hero");
        assert!(
            tag.starts_with("<span ") && tag.contains("role=\"status\""),
            "{tag}"
        );
        assert!(
            !html.contains("<a "),
            "the mark and note hold no link: {html}"
        );
    }

    #[test]
    fn marks_in_one_row_show_at_most_one_open_note() {
        fn view() -> Element {
            rsx! {
                RowNotesScope { open: Some("Healing".to_string()),
                    div { class: "match-card match-card-link",
                        a { class: "match-card-overlay", href: "/stats/match/x",
                            span { class: "visually-hidden", "View game" }
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
    fn a_result_is_a_word_on_a_soft_fill() {
        for (stored, word) in [
            ("victory", "Win"),
            ("WIN", "Win"),
            ("defeat", "Loss"),
            ("loss", "Loss"),
            ("draw", "Draw"),
            ("", "Draw"),
        ] {
            assert_eq!(outcome_word(stored), word, "{stored}");
        }
        fn view() -> Element {
            rsx! {
                {outcome_badge("m1", "match-outcome", "victory", false)}
                {outcome_badge("m2", "match-outcome", "defeat", false)}
                {outcome_badge("m3", "match-outcome", "draw", true)}
            }
        }
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert!(
            html.contains("<span class=\"match-outcome outcome-win\"><span class=\"stat-value-text ui-pill ui-pill--ok\">Win</span></span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"match-outcome outcome-loss\"><span class=\"stat-value-text ui-pill ui-pill--danger\">Loss</span></span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"stat-value-text ui-pill ui-pill--neutral\">Draw</span><button class=\"stat-unsure-mark\""),
            "a flagged result keeps the mark outside the pill: {html}"
        );
        assert!(
            html.contains("aria-controls=\"unsure-note-m3-result\""),
            "{html}"
        );
    }

    #[test]
    fn next_twice_before_the_page_lands_opens_it_once() {
        let mut trail = vec![None];
        assert!(push_next_page(&mut trail, "c2"));
        assert!(!push_next_page(&mut trail, "c2"), "same cursor again");
        assert_eq!(trail, vec![None, Some("c2".to_string())]);
        assert!(push_next_page(&mut trail, "c3"));
        assert_eq!(trail.len(), 3);
    }

    #[test]
    fn status_line_names_every_state() {
        assert_eq!(history_status(None, false, 0), "Loading match history.");
        // The failure card is the alert: the status line does not repeat it.
        assert_eq!(history_status(None, true, 0), "");
        assert_eq!(history_status(Some(0), false, 0), "No games recorded yet.");
        assert_eq!(
            history_status(Some(5), false, 0),
            "No games match these filters on this page."
        );
        assert_eq!(history_status(Some(1), false, 1), "Showing 1 game.");
        assert_eq!(
            history_status(Some(20), false, 7),
            "Showing 7 of 20 games on this page."
        );
    }

    #[test]
    fn game_dates_read_like_the_frames() {
        assert_eq!(game_date(&played_at()), "15 Mar 2026");
        let early = Utc.with_ymd_and_hms(2026, 11, 2, 0, 0, 0).unwrap();
        assert_eq!(game_date(&early), "2 Nov 2026");
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
