use dioxus::prelude::*;

use crate::components::DataTable;
use crate::components::charts::{BarEntry, HBarChart};
use crate::components::ui::ChipButton;
use crate::hooks::ApiResource;

use super::role::hero_row_matches_filter;
use super::{HeroStats, load_error_state, wr_bar_color, wr_text_class};
use crate::sample::{
    HERO_RULE_NOTE, OTHER_HEROES, OTHER_RESULT_CLASS, Tally, fold_small, other_label,
};

/// Sort key for the "Win %" sort: heroes with a win rate first (highest
/// first), then record-only heroes by games played.
fn wr_sort_key(h: &HeroStats) -> (bool, f64, u32) {
    let rate = h.tally().win_rate_pct();
    (rate.is_some(), rate.unwrap_or(0.0), h.matches)
}

pub(super) fn heroes_tab(
    heroes: ApiResource<Vec<HeroStats>>,
    mut role_filter: Signal<&'static str>,
    mut sort_by: Signal<&'static str>,
) -> Element {
    let err = heroes.error.read().clone();
    let data = heroes.data.read();
    let list = data.as_ref().and_then(|d| d.as_ref());
    match list {
        None if err.is_some() => load_error_state("hero stats", heroes.refresh),
        None => rsx! { p { class: "loading-state", "Loading hero stats..." } },
        Some(list) if list.is_empty() => rsx! {
            p { class: "empty-state", "No hero stats yet." }
        },
        Some(list) => {
            let role = role_filter();
            let sort = sort_by();

            let filtered: Vec<&HeroStats> = list
                .iter()
                .filter(|h| hero_row_matches_filter(&h.hero, role))
                .collect();

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
                div { class: "stats-filters",
                    div { class: "filter-group", role: "group", aria_labelledby: "heroes-filter-role",
                        span { class: "filter-label", id: "heroes-filter-role", "Current role" }
                        {["All", "Tank", "Damage", "Support"].iter().map(|&r| {
                            rsx! {
                                ChipButton {
                                    key: "{r}",
                                    class: "filter-chip".to_string(),
                                    label: r.to_string(),
                                    pressed: role == r,
                                    onclick: move |_| role_filter.set(r),
                                }
                            }
                        })}
                    }
                    div { class: "filter-group", role: "group", aria_labelledby: "heroes-filter-sort",
                        span { class: "filter-label", id: "heroes-filter-sort", "Sort" }
                        {[["matches", "Matches"], ["wr", "Win %"], ["elims", "Avg Elims"]].iter().map(|&[k, label]| {
                            rsx! {
                                ChipButton {
                                    key: "{k}",
                                    class: "filter-chip".to_string(),
                                    label: label.to_string(),
                                    pressed: sort == k,
                                    onclick: move |_| sort_by.set(k),
                                }
                            }
                        })}
                    }
                }

                if !top_heroes.is_empty() {
                    div { class: "heroes-chart-section",
                        div { class: "section-title", "Top Heroes by Win Rate (10+ games)" }
                        HBarChart {
                            entries: top_heroes,
                            max: Some(100.0),
                            reference: Some(50.0),
                        }
                    }
                }

                p { class: "stats-gate-note", {HERO_RULE_NOTE} }
                DataTable { headers: vec!["Hero", "Matches", "Win %", "Avg Elims", "Avg Deaths", "Avg Dmg", "Avg Heal"],
                    for hero in sorted.iter() {
                        {
                            let tally = hero.tally();
                            let result = tally.label();
                            let wr_cls = wr_text_class(tally);
                            rsx! {
                                tr { key: "{hero.hero}",
                                    td { "{hero.hero}" }
                                    td { "{hero.matches}" }
                                    td { span { class: "{wr_cls}", "{result}" } }
                                    td { "{hero.avg_elims:.1}" }
                                    td { "{hero.avg_deaths:.1}" }
                                    td { "{hero.avg_damage:.0}" }
                                    td { "{hero.avg_healing:.0}" }
                                }
                            }
                        }
                    }
                    if other_count > 0 {
                        tr { key: "{OTHER_HEROES}", class: "stats-row-muted", title: "{other_names}",
                            td { "{OTHER_HEROES} ({other_count})" }
                            td { "{other.games}" }
                            td { span { class: "{OTHER_RESULT_CLASS}", "{other_result}" } }
                            td { "-" }
                            td { "-" }
                            td { "-" }
                            td { "-" }
                        }
                    }
                }
            }
        }
    }
}
