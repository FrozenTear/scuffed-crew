use dioxus::prelude::*;

use crate::components::DataTable;
use crate::components::charts::{BarEntry, HBarChart};
use crate::hooks::ApiResource;

use super::overview::hero_to_role;
use super::{HeroStats, load_error_state, winrate_class, winrate_pct};

/// Single accent for WR bars (W2 will refine + 50% hairline); avoid 3-bin on bars.
fn wr_bar_accent() -> &'static str {
    "var(--accent)"
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
                .filter(|h| role == "All" || hero_to_role(&h.hero) == role)
                .collect();

            let mut sorted = filtered.clone();
            match sort {
                "wr" => sorted.sort_by(|a, b| {
                    let wa = winrate_pct(a.wins, a.matches);
                    let wb = winrate_pct(b.wins, b.matches);
                    wb.partial_cmp(&wa).unwrap_or(std::cmp::Ordering::Equal)
                }),
                "elims" => sorted.sort_by(|a, b| {
                    b.avg_elims
                        .partial_cmp(&a.avg_elims)
                        .unwrap_or(std::cmp::Ordering::Equal)
                }),
                _ => sorted.sort_by_key(|b| std::cmp::Reverse(b.matches)),
            }

            let mut by_wr: Vec<_> = filtered
                .iter()
                .copied()
                .filter(|h| h.matches >= 3)
                .collect();
            by_wr.sort_by(|a, b| {
                let wa = winrate_pct(a.wins, a.matches);
                let wb = winrate_pct(b.wins, b.matches);
                wb.partial_cmp(&wa).unwrap_or(std::cmp::Ordering::Equal)
            });
            let top_heroes: Vec<BarEntry> = by_wr
                .iter()
                .take(8)
                .map(|h| {
                    let wr = winrate_pct(h.wins, h.matches);
                    BarEntry {
                        label: h.hero.clone(),
                        value: wr,
                        color: wr_bar_accent().to_string(),
                        display: format!("{wr:.1}%"),
                    }
                })
                .collect();

            rsx! {
                div { class: "stats-filters",
                    div { class: "filter-group",
                        span { class: "filter-label", "Role" }
                        {["All", "Tank", "Damage", "Support"].iter().map(|&r| {
                            let active = role == r;
                            rsx! {
                                button {
                                    key: "{r}",
                                    class: if active { "filter-chip active" } else { "filter-chip" },
                                    onclick: move |_| role_filter.set(r),
                                    "{r}"
                                }
                            }
                        })}
                    }
                    div { class: "filter-group",
                        span { class: "filter-label", "Sort" }
                        {[["matches", "Matches"], ["wr", "Win %"], ["elims", "Avg Elims"]].iter().map(|&[k, label]| {
                            let active = sort == k;
                            rsx! {
                                button {
                                    key: "{k}",
                                    class: if active { "filter-chip active" } else { "filter-chip" },
                                    onclick: move |_| sort_by.set(k),
                                    "{label}"
                                }
                            }
                        })}
                    }
                }

                if !top_heroes.is_empty() {
                    div { class: "heroes-chart-section",
                        div { class: "section-title", "Top Heroes by Win Rate (3+ matches)" }
                        HBarChart { entries: top_heroes }
                    }
                }

                DataTable { headers: vec!["Hero", "Matches", "Win %", "Avg Elims", "Avg Deaths", "Avg Dmg", "Avg Heal"],
                    for hero in sorted.iter() {
                        {
                            let wr = winrate_pct(hero.wins, hero.matches);
                            let wr_cls = winrate_class(wr);
                            let low_n = hero.matches < 3;
                            let row_cls = if low_n { "stats-row-muted" } else { "" };
                            rsx! {
                                tr { key: "{hero.hero}", class: "{row_cls}",
                                    td { "{hero.hero}" }
                                    td { "{hero.matches}" }
                                    td { span { class: "{wr_cls}", "{wr:.1}%" } }
                                    td { "{hero.avg_elims:.1}" }
                                    td { "{hero.avg_deaths:.1}" }
                                    td { "{hero.avg_damage:.0}" }
                                    td { "{hero.avg_healing:.0}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
