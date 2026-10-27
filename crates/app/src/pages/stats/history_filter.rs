//! History tab helpers: row filters, the mode label, and the per-map table.
//!
//! Everything here works on the rows already loaded for the current page.
//! The match list has no queue field (competitive or quick play), so there
//! is no queue filter yet.

use dioxus::prelude::*;

use crate::sample::{MAP_RULE_NOTE, RECORD_MIN_GAMES, Tally};

use super::role::history_row_matches_role;
use super::{PersonalMatch, map_game_mode, wr_text_class};

/// Filter value that keeps every row.
pub(super) const ALL: &str = "all";

/// Mode labels in display order. Matches the Maps tab plus the two older
/// modes the map list still knows.
const MODE_ORDER: &[&str] = &[
    "Escort",
    "Hybrid",
    "Control",
    "Push",
    "Flashpoint",
    "Clash",
    "Payload Race",
    "Assault",
];

/// Label used for a row whose map name is blank.
pub(super) const UNKNOWN_MAP: &str = "Unknown map";

/// Caption over the per-map table.
pub(super) fn map_table_note() -> String {
    format!(
        "Games on this page that match the mode and role filters. {MAP_RULE_NOTE} Under {RECORD_MIN_GAMES} games is muted."
    )
}

/// Win, loss or draw for one stored outcome. `victory` and `defeat` are
/// what the tracker stores; `win` and `loss` are accepted aliases. Anything
/// else counts as a draw, the same as the outcome filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    Win,
    Loss,
    Draw,
}

pub(super) fn outcome_kind(outcome: &str) -> Outcome {
    let o = outcome.trim();
    if o.eq_ignore_ascii_case("victory") || o.eq_ignore_ascii_case("win") {
        Outcome::Win
    } else if o.eq_ignore_ascii_case("defeat") || o.eq_ignore_ascii_case("loss") {
        Outcome::Loss
    } else {
        Outcome::Draw
    }
}

/// `true` when `outcome` passes the outcome chip `filter`
/// (`all`, `win`, `loss` or `draw`).
pub(super) fn outcome_matches(outcome: &str, filter: &str) -> bool {
    match filter {
        ALL => true,
        "win" => outcome_kind(outcome) == Outcome::Win,
        "loss" => outcome_kind(outcome) == Outcome::Loss,
        "draw" => outcome_kind(outcome) == Outcome::Draw,
        other => outcome.eq_ignore_ascii_case(other),
    }
}

/// Display label for a row's mode. A known mode is written the usual way
/// (`control` reads `Control`). A blank mode falls back to the map's mode.
/// Anything else is shown as stored.
pub(super) fn mode_label(game_mode: &str, map_name: &str) -> String {
    let raw = game_mode.trim();
    if raw.is_empty() {
        return map_game_mode(map_name).to_string();
    }
    MODE_ORDER
        .iter()
        .find(|known| known.eq_ignore_ascii_case(raw))
        .map(|known| (*known).to_string())
        .unwrap_or_else(|| raw.to_string())
}

/// `true` when the row passes the mode chip `filter` (`all` or a label from
/// [`mode_label`]).
pub(super) fn mode_matches(row: &PersonalMatch, filter: &str) -> bool {
    filter == ALL || mode_label(&row.game_mode, &row.map_name).eq_ignore_ascii_case(filter)
}

/// Rows that pass all three filters, in page order.
pub(super) fn filter_rows<'a>(
    rows: &'a [PersonalMatch],
    outcome: &str,
    role: &str,
    mode: &str,
) -> Vec<&'a PersonalMatch> {
    rows.iter()
        .filter(|m| {
            outcome_matches(&m.outcome, outcome)
                && history_row_matches_role(&m.role, &m.hero, role)
                && mode_matches(m, mode)
        })
        .collect()
}

/// Mode chips for this page: every mode on the loaded rows, plus the active
/// one so it can still be cleared after paging. Known modes come first in
/// [`MODE_ORDER`], then the rest by name.
pub(super) fn page_modes(rows: &[PersonalMatch], active: &str) -> Vec<String> {
    let mut modes: Vec<String> = Vec::new();
    let labels = rows
        .iter()
        .map(|m| mode_label(&m.game_mode, &m.map_name))
        .chain((active != ALL).then(|| active.to_string()));
    for label in labels {
        if !modes.iter().any(|m| m.eq_ignore_ascii_case(&label)) {
            modes.push(label);
        }
    }
    let rank = |label: &str| {
        MODE_ORDER
            .iter()
            .position(|known| *known == label)
            .unwrap_or(MODE_ORDER.len())
    };
    modes.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
    modes
}

/// One line of the per-map table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MapRow {
    pub(super) map: String,
    pub(super) mode: String,
    pub(super) tally: Tally,
}

/// Games and results per map over `rows`. Map names group case-blind and
/// keep the first spelling seen. Sorted by games, most first, then by name.
pub(super) fn map_rows<'a>(rows: impl IntoIterator<Item = &'a PersonalMatch>) -> Vec<MapRow> {
    let mut out: Vec<MapRow> = Vec::new();
    for m in rows {
        let name = m.map_name.trim();
        let name = if name.is_empty() { UNKNOWN_MAP } else { name };
        let idx = match out.iter().position(|r| r.map.eq_ignore_ascii_case(name)) {
            Some(idx) => idx,
            None => {
                out.push(MapRow {
                    map: name.to_string(),
                    mode: mode_label(&m.game_mode, &m.map_name),
                    tally: Tally::default(),
                });
                out.len() - 1
            }
        };
        let t = &mut out[idx].tally;
        t.games += 1;
        match outcome_kind(&m.outcome) {
            Outcome::Win => t.wins += 1,
            Outcome::Loss => t.losses += 1,
            Outcome::Draw => t.draws += 1,
        }
    }
    out.sort_by(|a, b| {
        b.tally
            .games
            .cmp(&a.tally.games)
            .then_with(|| a.map.to_lowercase().cmp(&b.map.to_lowercase()))
    });
    out
}

/// Result cell for one map, following [`crate::sample`]: under 3 games the
/// row is muted, 3 to 9 games shows the record, 10 or more the win rate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MapResult {
    pub(super) text: String,
    pub(super) row_muted: bool,
    pub(super) text_class: &'static str,
}

pub(super) fn map_result(tally: Tally) -> MapResult {
    MapResult {
        text: tally.label(),
        row_muted: tally.games < RECORD_MIN_GAMES,
        text_class: wr_text_class(tally),
    }
}

/// Per-map table under the history list. Nothing renders for no rows.
pub(super) fn map_table(rows: &[MapRow]) -> Element {
    if rows.is_empty() {
        return rsx! {};
    }
    let note = map_table_note();
    rsx! {
        section { class: "history-maps",
            h3 { class: "section-title", "Map results" }
            p { class: "stats-gate-note", "{note}" }
            div { class: "data-table-scroll",
                table { class: "data-table history-map-table",
                    thead {
                        tr {
                            th { "Map" }
                            th { "Mode" }
                            th { "Games" }
                            th { "Result" }
                        }
                    }
                    tbody {
                        for r in rows.iter() {
                            {
                                let res = map_result(r.tally);
                                let row_class = if res.row_muted { "stats-row-muted" } else { "" };
                                rsx! {
                                    tr { key: "{r.map}", class: "{row_class}",
                                        td { "{r.map}" }
                                        td { "{r.mode}" }
                                        td { "{r.tally.games}" }
                                        td {
                                            span { class: "{res.text_class}", "{res.text}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Styles for the compact history rows and the per-map table.
pub(super) const HISTORY_CSS: &str = r#"
    .match-card .match-identity.match-identity-compact {
        display: flex;
        flex-wrap: wrap;
        align-items: baseline;
        column-gap: 0.6rem;
        row-gap: 0.1rem;
    }
    .match-identity-compact .match-hero-line { flex: 0 1 auto; }
    .match-identity-compact .match-map { flex: 1 1 10rem; }
    .match-mode {
        flex: 0 0 auto;
        white-space: nowrap;
    }
    .stats-page[data-density="compact"] .match-card.match-row-compact {
        padding-top: 0.3rem;
        padding-bottom: 0.3rem;
    }
    .history-maps { margin-top: 1.75rem; }
    .history-maps .data-table { min-width: 0; }
    .history-maps .data-table td,
    .history-maps .data-table th { padding: 0.4rem 0.6rem; }
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn row(map: &str, mode: &str, role: &str, hero: &str, outcome: &str) -> PersonalMatch {
        PersonalMatch {
            id: format!("{map}-{outcome}"),
            hero: hero.to_string(),
            map_name: map.to_string(),
            game_mode: mode.to_string(),
            role: role.to_string(),
            outcome: outcome.to_string(),
            elims: 1,
            deaths: 2,
            assists: 3,
            damage: 4,
            healing: 5,
            mitigation: 0,
            played_at: Utc.with_ymd_and_hms(2026, 3, 15, 12, 0, 0).unwrap(),
            recognizer: None,
            suspect_fields: None,
        }
    }

    #[test]
    fn outcome_kinds_and_filter() {
        assert_eq!(outcome_kind("VICTORY"), Outcome::Win);
        assert_eq!(outcome_kind("win"), Outcome::Win);
        assert_eq!(outcome_kind("defeat"), Outcome::Loss);
        assert_eq!(outcome_kind("Loss"), Outcome::Loss);
        assert_eq!(outcome_kind("draw"), Outcome::Draw);
        assert_eq!(outcome_kind(""), Outcome::Draw);
        assert!(outcome_matches("defeat", ALL));
        assert!(outcome_matches("victory", "win"));
        assert!(!outcome_matches("victory", "loss"));
        assert!(outcome_matches("defeat", "loss"));
        assert!(outcome_matches("draw", "draw"));
        assert!(outcome_matches("tie", "draw"));
        assert!(!outcome_matches("win", "draw"));
    }

    #[test]
    fn mode_label_normalises_and_falls_back_to_the_map() {
        assert_eq!(mode_label("control", "Ilios"), "Control");
        assert_eq!(mode_label("  PUSH ", "Colosseo"), "Push");
        assert_eq!(mode_label("payload race", "x"), "Payload Race");
        assert_eq!(mode_label("", "Ilios"), "Control");
        assert_eq!(mode_label("", "Watchpoint: Gibraltar"), "Escort");
        assert_eq!(mode_label("", "not a map"), "Other");
        assert_eq!(mode_label("Weird", "Ilios"), "Weird");
    }

    #[test]
    fn filters_combine_outcome_role_and_mode() {
        let rows = vec![
            row("Ilios", "Control", "Support", "Ana", "victory"),
            row("Havana", "Escort", "Tank", "Reinhardt", "defeat"),
            row("Nepal", "control", "Support", "Kiriko", "defeat"),
            row("Busan", "", "Damage", "Tracer", "draw"),
        ];
        let ids = |v: Vec<&PersonalMatch>| v.iter().map(|m| m.id.clone()).collect::<Vec<_>>();
        assert_eq!(filter_rows(&rows, ALL, ALL, ALL).len(), 4);
        assert_eq!(
            ids(filter_rows(&rows, ALL, ALL, "Control")),
            vec!["Ilios-victory", "Nepal-defeat", "Busan-draw"]
        );
        assert_eq!(
            ids(filter_rows(&rows, "loss", ALL, "Control")),
            vec!["Nepal-defeat"]
        );
        assert_eq!(
            ids(filter_rows(&rows, ALL, "Tank", ALL)),
            vec!["Havana-defeat"]
        );
        assert!(filter_rows(&rows, "win", "Tank", ALL).is_empty());
        assert!(filter_rows(&rows, ALL, ALL, "Push").is_empty());
    }

    #[test]
    fn page_modes_are_ordered_and_keep_the_active_one() {
        let rows = vec![
            row("Ilios", "control", "Support", "Ana", "victory"),
            row("Havana", "Escort", "Tank", "Reinhardt", "defeat"),
            row("Nepal", "Control", "Support", "Kiriko", "defeat"),
            row("x", "Zeta", "Support", "Ana", "victory"),
        ];
        assert_eq!(page_modes(&rows, ALL), vec!["Escort", "Control", "Zeta"]);
        assert_eq!(
            page_modes(&rows, "Push"),
            vec!["Escort", "Control", "Push", "Zeta"]
        );
        assert_eq!(
            page_modes(&rows, "Control"),
            vec!["Escort", "Control", "Zeta"]
        );
        assert!(page_modes(&[], ALL).is_empty());
    }

    #[test]
    fn map_rows_count_results_and_sort_by_games() {
        let rows = vec![
            row("Ilios", "Control", "Support", "Ana", "victory"),
            row("Havana", "Escort", "Tank", "Reinhardt", "defeat"),
            row("ilios", "Control", "Support", "Ana", "defeat"),
            row("Ilios", "Control", "Support", "Ana", "draw"),
            row("Busan", "Control", "Support", "Ana", "victory"),
            row("  ", "", "Support", "Ana", "victory"),
        ];
        let out = map_rows(rows.iter());
        let names: Vec<&str> = out.iter().map(|r| r.map.as_str()).collect();
        assert_eq!(names, vec!["Ilios", "Busan", "Havana", UNKNOWN_MAP]);
        assert_eq!(out[0].tally, Tally::new(3, 1, 1, 1));
        assert_eq!(out[0].mode, "Control");
        assert_eq!(out[2].tally, Tally::new(1, 0, 1, 0));
        assert_eq!(out[3].mode, "Other");
        assert!(map_rows(std::iter::empty()).is_empty());
    }

    #[test]
    fn map_result_follows_the_small_sample_rule() {
        let two = map_result(Tally::new(2, 1, 1, 0));
        assert_eq!(two.text, "1W-1L");
        assert!(two.row_muted);
        assert!(two.text_class.contains("muted"));

        let nine = map_result(Tally::new(9, 5, 3, 1));
        assert_eq!(nine.text, "5W-3L-1D");
        assert!(!nine.row_muted);
        assert!(!nine.text.contains('%'));

        let ten = map_result(Tally::new(10, 7, 3, 0));
        assert_eq!(ten.text, "70.0%");
        assert!(!ten.row_muted);
        assert_eq!(ten.text_class, "stats-wr");
    }

    fn render(app: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn map_table_renders_rows_in_order_and_mutes_small_maps() {
        fn view() -> Element {
            let mut rows = Vec::new();
            for _ in 0..10 {
                rows.push(row("Ilios", "Control", "Support", "Ana", "victory"));
            }
            for _ in 0..4 {
                rows.push(row("Havana", "Escort", "Tank", "Reinhardt", "defeat"));
            }
            rows.push(row("Busan", "Control", "Support", "Ana", "draw"));
            map_table(&map_rows(rows.iter()))
        }
        fn empty() -> Element {
            map_table(&[])
        }
        let html = render(view);
        let ilios = html.find("Ilios").expect("Ilios");
        let havana = html.find("Havana").expect("Havana");
        let busan = html.find("Busan").expect("Busan");
        assert!(ilios < havana && havana < busan, "{html}");
        assert!(html.contains("100.0%"), "{html}");
        assert!(html.contains("0W-4L"), "{html}");
        assert!(html.contains("0W-0L-1D"), "{html}");
        assert_eq!(html.matches("stats-row-muted").count(), 1, "{html}");
        assert!(html.contains("Map results"), "{html}");
        assert!(!render(empty).contains("history-maps"));
    }

    #[test]
    fn copy_has_no_long_dashes() {
        let note = map_table_note();
        for text in [note.as_str(), HISTORY_CSS] {
            assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'));
        }
    }
}
