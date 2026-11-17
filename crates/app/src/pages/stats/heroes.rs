use dioxus::prelude::*;

use crate::components::charts::{BarEntry, HBarChart};
use crate::components::ui::{ChipButton, ChipRow, EmptyCard, SkeletonBars, btn_class};
use crate::hooks::ApiResource;
use crate::pages::stats_member::{
    Col, STATS_VIEWS_CSS, STATS_VIEWS_CSS_ID, StatsTable, rate_cell, stats_load_error,
};
use crate::routes::Route;

use super::role::hero_row_matches_filter;
use super::{HeroStats, wr_bar_color, wr_text_class};
use crate::sample::{
    HERO_RULE_NOTE, OTHER_HEROES, OTHER_RESULT_CLASS, Tally, fold_small, other_label,
};

/// Sort key for the "Win %" sort: heroes with a win rate first (highest
/// first), then record-only heroes by games played.
fn wr_sort_key(h: &HeroStats) -> (bool, f64, u32) {
    let rate = h.tally().win_rate_pct();
    (rate.is_some(), rate.unwrap_or(0.0), h.matches)
}

/// Role chips of the Heroes tab, in display order.
const ROLE_CHIPS: [&str; 4] = ["All", "Tank", "Damage", "Support"];
/// Sort chips of the Heroes tab: key and label.
const SORT_CHIPS: [[&str; 2]; 3] = [
    ["matches", "Matches"],
    ["wr", "Win rate"],
    ["elims", "Avg elims"],
];

/// What screen readers hear for the tab. Always rendered, only the text
/// changes.
fn heroes_status(loaded: Option<usize>, failed: bool, shown: usize) -> String {
    match loaded {
        None if failed => "Couldn't load hero stats.".to_string(),
        None => "Loading hero stats.".to_string(),
        Some(0) => "No hero stats yet.".to_string(),
        Some(_) if shown == 0 => "No heroes match this role.".to_string(),
        Some(_) if shown == 1 => "Showing 1 hero.".to_string(),
        Some(_) => format!("Showing {shown} heroes."),
    }
}

/// "No hero stats yet", with the next step: connect the tracker.
#[component]
fn HeroesEmpty() -> Element {
    rsx! {
        EmptyCard {
            title: "No hero stats yet",
            message: "Hero results show up here after the tracker uploads your first games.",
            actions: rsx! {
                Link { to: Route::StatsTokens {}, class: btn_class(true), "Set up the tracker" }
            },
        }
    }
}

pub(super) fn heroes_tab(
    heroes: ApiResource<Vec<HeroStats>>,
    mut role_filter: Signal<&'static str>,
    mut sort_by: Signal<&'static str>,
) -> Element {
    let failed = heroes.error.read().is_some();
    let data = heroes.data.read();
    let list = data.as_ref().and_then(|d| d.as_ref());
    let role = role_filter();
    let sort = sort_by();

    let filtered: Vec<&HeroStats> = list
        .map(|list| {
            list.iter()
                .filter(|h| hero_row_matches_filter(&h.hero, role))
                .collect()
        })
        .unwrap_or_default();
    let status = heroes_status(list.map(Vec::len), failed, filtered.len());

    let body = match list {
        None if failed => stats_load_error("hero stats", heroes.refresh),
        None => rsx! { SkeletonBars { rows: 4 } },
        Some(list) if list.is_empty() => rsx! { HeroesEmpty {} },
        Some(_) => {
            // Under 3 games a hero folds into one "Other heroes" row.
            let (kept, folded) = fold_small(&filtered, |h| h.tally());
            let other = Tally::sum(folded.iter().map(|h| h.tally()));
            let other_names = folded
                .iter()
                .map(|h| h.hero.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let other_count = folded.len();
            // The pool mixes heroes, so it always shows the record, muted.
            let other_result = other_label(other);

            let mut sorted = kept.clone();
            match sort {
                "wr" => sorted.sort_by(|a, b| {
                    wr_sort_key(b)
                        .partial_cmp(&wr_sort_key(a))
                        .unwrap_or(std::cmp::Ordering::Equal)
                }),
                "elims" => sorted.sort_by(|a, b| {
                    b.avg_elims
                        .partial_cmp(&a.avg_elims)
                        .unwrap_or(std::cmp::Ordering::Equal)
                }),
                _ => sorted.sort_by_key(|b| std::cmp::Reverse(b.matches)),
            }

            let mut by_wr: Vec<(&HeroStats, f64)> = kept
                .iter()
                .filter_map(|h| h.tally().win_rate_pct().map(|wr| (*h, wr)))
                .collect();
            by_wr.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            // The bar repeats the number written next to it, so the fill
            // colour is never the only signal.
            let top_heroes: Vec<BarEntry> = by_wr
                .iter()
                .take(8)
                .map(|&(h, wr)| BarEntry {
                    label: h.hero.clone(),
                    value: wr,
                    color: wr_bar_color(h.tally()).to_string(),
                    display: format!("{wr:.1}% ({} games)", h.matches),
                    muted: false,
                })
                .collect();

            rsx! {
                div { class: "stats-view-toolbar",
                    div { class: "stats-view-filter",
                        span { class: "stats-view-filter-label", aria_hidden: "true", "Current role" }
                        ChipRow { label: "Current role", scroll: true,
                            {ROLE_CHIPS.iter().map(|&r| {
                                rsx! {
                                    ChipButton {
                                        key: "{r}",
                                        class: "chip".to_string(),
                                        label: r.to_string(),
                                        pressed: role == r,
                                        onclick: move |_| role_filter.set(r),
                                    }
                                }
                            })}
                        }
                    }
                    div { class: "stats-view-filter",
                        span { class: "stats-view-filter-label", aria_hidden: "true", "Sort by" }
                        ChipRow { label: "Sort by", scroll: true,
                            {SORT_CHIPS.iter().map(|&[k, label]| {
                                rsx! {
                                    ChipButton {
                                        key: "{k}",
                                        class: "chip".to_string(),
                                        label: label.to_string(),
                                        pressed: sort == k,
                                        onclick: move |_| sort_by.set(k),
                                    }
                                }
                            })}
                        }
                    }
                }

                if !top_heroes.is_empty() {
                    section { class: "ui-stack", aria_labelledby: "heroes-chart-title",
                        h2 { id: "heroes-chart-title", class: "ui-section-title", "Top heroes by win rate" }
                        p { class: "stats-view-note", "Heroes with 10 or more decided games. The line marks 50%." }
                        div { class: "stats-view-card",
                            HBarChart {
                                entries: top_heroes,
                                max: Some(100.0),
                                reference: Some(50.0),
                            }
                        }
                    }
                }

                section { class: "ui-stack", aria_labelledby: "heroes-table-title",
                    h2 { id: "heroes-table-title", class: "ui-section-title", "All heroes" }
                    p { class: "stats-view-note", {HERO_RULE_NOTE} }
                    if sorted.is_empty() && other_count == 0 {
                        p { class: "stats-view-note", "No heroes match this role. Pick All to see every hero." }
                    } else {
                        StatsTable {
                            labelled_by: "heroes-table-title",
                            wide: true,
                            cols: vec![
                                Col::Text("Hero"),
                                Col::Num("Matches"),
                                Col::Rate("Win rate"),
                                Col::Num("Avg elims"),
                                Col::Num("Avg deaths"),
                                Col::Num("Avg damage"),
                                Col::Num("Avg healing"),
                            ],
                            for hero in sorted.iter() {
                                {
                                    let tally = hero.tally();
                                    let result = tally.label();
                                    rsx! {
                                        tr { key: "{hero.hero}",
                                            th { scope: "row", "{hero.hero}" }
                                            td { class: "num", "{hero.matches}" }
                                            {rate_cell(tally, &result, wr_text_class(tally))}
                                            td { class: "num", "{hero.avg_elims:.1}" }
                                            td { class: "num", "{hero.avg_deaths:.1}" }
                                            td { class: "num", "{hero.avg_damage:.0}" }
                                            td { class: "num", "{hero.avg_healing:.0}" }
                                        }
                                    }
                                }
                            }
                            if other_count > 0 {
                                tr { key: "{OTHER_HEROES}", class: "stats-row-muted", title: "{other_names}",
                                    th { scope: "row", "{OTHER_HEROES} ({other_count})" }
                                    td { class: "num", "{other.games}" }
                                    td { class: "rate", span { class: "{OTHER_RESULT_CLASS}", "{other_result}" } }
                                    td { class: "num", "-" }
                                    td { class: "num", "-" }
                                    td { class: "num", "-" }
                                    td { class: "num", "-" }
                                }
                            }
                        }
                    }
                }
            }
        }
    };

    rsx! {
        crate::styles::StyleOnce { id: STATS_VIEWS_CSS_ID, css: STATS_VIEWS_CSS }
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

    fn hero(name: &str, matches: u32, wins: u32) -> HeroStats {
        HeroStats {
            hero: name.to_string(),
            matches,
            wins,
            losses: matches - wins,
            draws: 0,
            avg_elims: 12.34,
            avg_deaths: 5.0,
            avg_damage: 6000.4,
            avg_healing: 3000.0,
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

    fn render(app: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        for _ in 0..8 {
            dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        }
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn status_line_names_every_state() {
        assert_eq!(heroes_status(None, false, 0), "Loading hero stats.");
        assert_eq!(heroes_status(None, true, 0), "Couldn't load hero stats.");
        assert_eq!(heroes_status(Some(0), false, 0), "No hero stats yet.");
        assert_eq!(
            heroes_status(Some(4), false, 0),
            "No heroes match this role."
        );
        assert_eq!(heroes_status(Some(4), false, 1), "Showing 1 hero.");
        assert_eq!(heroes_status(Some(4), false, 3), "Showing 3 heroes.");
    }

    #[test]
    fn tab_has_named_chip_groups_a_labelled_table_and_no_heading_above_h2() {
        fn view() -> Element {
            let heroes = resource_of(Some(vec![
                hero("Ana", 12, 8),
                hero("Kiriko", 5, 2),
                hero("Mercy", 1, 1),
            ]));
            let role = use_signal(|| "All");
            let sort = use_signal(|| "matches");
            heroes_tab(heroes, role, sort)
        }
        let html = render(view);
        assert!(!html.contains("<h1") && !html.contains("<main"), "{html}");
        assert!(
            html.contains("<p class=\"visually-hidden\" role=\"status\">Showing 3 heroes.</p>"),
            "{html}"
        );
        for group in ["Current role", "Sort by"] {
            assert!(
                html.contains(&format!("role=\"group\" aria-label=\"{group}\"")),
                "{group}: {html}"
            );
        }
        assert!(
            html.contains("aria-pressed=\"true\" class=\"chip active\">All<"),
            "{html}"
        );
        assert!(
            html.contains("aria-pressed=\"false\" class=\"chip\">Win rate<"),
            "{html}"
        );
        assert!(
            html.contains("role=\"region\" aria-labelledby=\"heroes-table-title\" tabindex=\"0\""),
            "{html}"
        );
        assert!(html.contains("id=\"heroes-table-title\""), "{html}");
        // A rate is written as a number, the record below the gate, and the
        // folded pool keeps its own muted row.
        assert!(
            html.contains("<span class=\"stats-wr\">66.7%</span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"stats-wr muted\">2W-3L</span>"),
            "{html}"
        );
        assert!(html.contains("Other heroes (1)"), "{html}");
        assert!(
            html.contains("66.7% (12 games)"),
            "the bar carries its number: {html}"
        );
        assert!(
            !html.contains('\u{2014}') && !html.contains('\u{2013}'),
            "{html}"
        );
    }

    #[test]
    fn loading_shows_bars_and_a_spoken_line() {
        fn view() -> Element {
            let heroes = resource_of(None::<Vec<HeroStats>>);
            // `resource_of(None)` resolves to no rows: the fetch is still open.
            let role = use_signal(|| "All");
            let sort = use_signal(|| "matches");
            heroes_tab(heroes, role, sort)
        }
        let html = render(view);
        assert!(
            html.contains("role=\"status\">Loading hero stats.</p>"),
            "{html}"
        );
        assert!(html.contains("ui-skel-list"), "{html}");
    }
}
