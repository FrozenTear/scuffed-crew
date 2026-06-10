use dioxus::prelude::*;

use serde::Deserialize;

use crate::components::{DataTable, SummaryCard};
use crate::hooks::use_api_with;

#[derive(Debug, Clone, Deserialize)]
struct PersonalStats {
    #[allow(dead_code)]
    member_id: String,
    total_matches: u32,
    wins: u32,
    losses: u32,
    draws: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct HeroStats {
    hero: String,
    matches: u32,
    wins: u32,
    #[allow(dead_code)]
    losses: u32,
    #[allow(dead_code)]
    draws: u32,
    avg_elims: f64,
    avg_deaths: f64,
    avg_damage: f64,
    avg_healing: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct MapStats {
    map_name: String,
    matches: u32,
    wins: u32,
    losses: u32,
    draws: u32,
}

fn winrate_pct(wins: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        (wins as f64 / total as f64) * 100.0
    }
}

fn winrate_class(pct: f64) -> &'static str {
    if pct >= 55.0 {
        "stats-winrate high"
    } else if pct >= 45.0 {
        "stats-winrate mid"
    } else {
        "stats-winrate low"
    }
}

#[derive(Clone, Copy, PartialEq)]
enum MemberStatsTab {
    Overview,
    Heroes,
    Maps,
}

const MEMBER_STATS_CSS: &str = r#"
    .stats-page {
        max-width: 1100px;
        margin: 0 auto;
        padding: 2rem 1.5rem;
    }
    .stats-page h1 {
        font-family: var(--font-head);
        font-size: 1.8rem;
        color: var(--text);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        margin-bottom: 1.5rem;
    }
    .stats-tabs {
        display: flex;
        gap: 0.25rem;
        margin-bottom: 1.5rem;
        border-bottom: 1px solid var(--border);
    }
    .stats-tab {
        padding: 0.6rem 1.2rem;
        border: none;
        background: none;
        color: var(--text-2);
        font-family: var(--font-head);
        font-size: 0.9rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.03em;
        cursor: pointer;
        border-bottom: 2px solid transparent;
        margin-bottom: -1px;
        transition: color 0.15s, border-color 0.15s;
    }
    .stats-tab:hover { color: var(--text); }
    .stats-tab.active {
        color: var(--accent);
        border-bottom-color: var(--accent);
    }
    .stats-winrate {
        display: inline-block;
        padding: 0.1rem 0.4rem;
        border-radius: 4px;
        font-size: 0.8rem;
        font-weight: 600;
    }
    .stats-winrate.high { color: var(--ok); }
    .stats-winrate.mid { color: var(--warn); }
    .stats-winrate.low { color: var(--danger); }
"#;

#[component]
pub fn StatsMember(id: String) -> Element {
    let member_id = id.clone();
    let member_id_h = id.clone();
    let member_id_m = id.clone();

    let stats = use_api_with::<PersonalStats>(move || format!("/api/stats/member/{member_id}"));

    let heroes =
        use_api_with::<Vec<HeroStats>>(move || format!("/api/stats/member/{member_id_h}/heroes"));

    let maps =
        use_api_with::<Vec<MapStats>>(move || format!("/api/stats/member/{member_id_m}/maps"));

    let mut tab = use_signal(|| MemberStatsTab::Overview);

    rsx! {
        style { {MEMBER_STATS_CSS} }
        style { {crate::styles::admin::CSS} }

        div { class: "stats-page",
            h1 { "Member Stats" }

            {
                let data = stats.data.read();
                let s = data.as_ref().and_then(|d| d.as_ref());
                match s {
                    None => rsx! { p { class: "loading-state", "Loading stats..." } },
                    Some(s) => {
                        let wr = winrate_pct(s.wins, s.total_matches);
                        rsx! {
                            div { class: "summary-cards",
                                SummaryCard { value: s.total_matches.to_string(), label: "Matches" }
                                SummaryCard { value: s.wins.to_string(), label: "Wins" }
                                SummaryCard { value: s.losses.to_string(), label: "Losses" }
                                SummaryCard { value: s.draws.to_string(), label: "Draws" }
                                SummaryCard { value: format!("{wr:.1}%"), label: "Win Rate" }
                            }
                        }
                    }
                }
            }

            div { class: "stats-tabs",
                button {
                    class: if tab() == MemberStatsTab::Overview { "stats-tab active" } else { "stats-tab" },
                    onclick: move |_| tab.set(MemberStatsTab::Overview),
                    "Overview"
                }
                button {
                    class: if tab() == MemberStatsTab::Heroes { "stats-tab active" } else { "stats-tab" },
                    onclick: move |_| tab.set(MemberStatsTab::Heroes),
                    "Heroes"
                }
                button {
                    class: if tab() == MemberStatsTab::Maps { "stats-tab active" } else { "stats-tab" },
                    onclick: move |_| tab.set(MemberStatsTab::Maps),
                    "Maps"
                }
            }

            match tab() {
                MemberStatsTab::Overview => rsx! {
                    p { class: "empty-state",
                        style: "padding: 1rem 0; text-align: left;",
                        "Select Heroes or Maps to view detailed breakdown."
                    }
                },
                MemberStatsTab::Heroes => rsx! {
                    {
                        let data = heroes.data.read();
                        let list = data.as_ref().and_then(|d| d.as_ref());
                        match list {
                            None => rsx! { p { class: "loading-state", "Loading hero stats..." } },
                            Some(list) if list.is_empty() => rsx! {
                                p { class: "empty-state", "No hero stats recorded." }
                            },
                            Some(list) => {
                                let mut sorted: Vec<_> = list.iter().collect();
                                sorted.sort_by_key(|b| std::cmp::Reverse(b.matches));
                                rsx! {
                                    DataTable { headers: vec!["Hero", "Matches", "Win %", "Avg Elims", "Avg Deaths", "Avg Dmg", "Avg Heal"],
                                        for hero in sorted.iter() {
                                            {
                                                let wr = winrate_pct(hero.wins, hero.matches);
                                                let wr_cls = winrate_class(wr);
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
                            },
                        }
                    }
                },
                MemberStatsTab::Maps => rsx! {
                    {
                        let data = maps.data.read();
                        let list = data.as_ref().and_then(|d| d.as_ref());
                        match list {
                            None => rsx! { p { class: "loading-state", "Loading map stats..." } },
                            Some(list) if list.is_empty() => rsx! {
                                p { class: "empty-state", "No map stats recorded." }
                            },
                            Some(list) => {
                                let mut sorted: Vec<_> = list.iter().collect();
                                sorted.sort_by_key(|b| std::cmp::Reverse(b.matches));
                                rsx! {
                                    DataTable { headers: vec!["Map", "Matches", "Win %", "Wins", "Losses", "Draws"],
                                        for map in sorted.iter() {
                                            {
                                                let wr = winrate_pct(map.wins, map.matches);
                                                let wr_cls = winrate_class(wr);
                                                rsx! {
                                                    tr { key: "{map.map_name}",
                                                        td { "{map.map_name}" }
                                                        td { "{map.matches}" }
                                                        td { span { class: "{wr_cls}", "{wr:.1}%" } }
                                                        td { "{map.wins}" }
                                                        td { "{map.losses}" }
                                                        td { "{map.draws}" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            },
                        }
                    }
                },
            }
        }
    }
}
