use dioxus::prelude::*;

use crate::components::DataTable;
use crate::components::charts::{BarEntry, HBarChart};
use crate::hooks::ApiResource;

use super::{HeroStats, MIN_GAMES, MIN_GAMES_NOTE, load_error_state, winrate_pct, wr_text_class};

pub(super) fn heroes_tab(heroes: ApiResource<Vec<HeroStats>>) -> Element {
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
            let mut sorted: Vec<_> = list.iter().collect();
            sorted.sort_by_key(|b| std::cmp::Reverse(b.matches));

            // Top heroes by win rate (min-games gate, top 8)
            let mut by_wr: Vec<_> = list.iter().filter(|h| h.matches >= MIN_GAMES).collect();
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
                        color: "var(--chart-wr)".to_string(),
                        display: format!("{wr:.1}% ({} games)", h.matches),
                        muted: false,
                    }
                })
                .collect();

            rsx! {
                if !top_heroes.is_empty() {
                    div { class: "heroes-chart-section",
                        div { class: "section-title", "Top Heroes by Win Rate (3+ matches)" }
                        HBarChart {
                            entries: top_heroes,
                            max: Some(100.0),
                            reference: Some(50.0),
                        }
                    }
                }

                p { class: "stats-gate-note", {MIN_GAMES_NOTE} }
                DataTable { headers: vec!["Hero", "Matches", "Win %", "Avg Elims", "Avg Deaths", "Avg Dmg", "Avg Heal"],
                    for hero in sorted.iter() {
                        {
                            let wr = winrate_pct(hero.wins, hero.matches);
                            let wr_cls = wr_text_class(hero.matches);
                            rsx! {
                                tr { key: "{hero.hero}",
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
