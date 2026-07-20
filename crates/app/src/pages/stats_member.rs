use dioxus::prelude::*;

use serde::Deserialize;

use crate::components::DataTable;
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

/// Match personal stats: plain WR text; mute low sample (no 3-bin traffic light).
fn wr_text_class(matches: u32) -> &'static str {
    if matches < 3 {
        "stats-wr muted"
    } else {
        "stats-wr"
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
        overflow-x: auto;
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
        white-space: nowrap;
    }
    .stats-tab:hover { color: var(--text); }
    .stats-tab.active {
        color: var(--accent);
        border-bottom-color: var(--accent);
    }
    .stats-summary {
        display: grid;
        grid-template-columns: minmax(220px, 2fr) minmax(140px, 1fr);
        gap: 1rem;
        margin-bottom: 2rem;
    }
    .stat-tile {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.25rem;
        text-align: center;
        display: flex;
        flex-direction: column;
        justify-content: center;
    }
    .stat-tile-value {
        font-family: var(--font-head);
        font-size: 1.9rem;
        color: var(--text);
        letter-spacing: 2px;
        line-height: 1;
    }
    .stat-tile-hero .stat-tile-value {
        font-size: 2.8rem;
        color: var(--accent);
    }
    .stat-tile-label {
        font-size: 0.75rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.05em;
        margin-top: 0.35rem;
    }
    .stat-tile-record {
        font-size: 0.9rem;
        color: var(--text-2);
        margin-top: 0.4rem;
    }
    .stats-wr { font-weight: 600; color: var(--text); font-variant-numeric: tabular-nums; }
    .stats-wr.muted { color: var(--text-3); font-weight: 400; }
    .stats-row-muted { opacity: 0.55; }
    .stats-page .data-table th:not(:first-child),
    .stats-page .data-table td:not(:first-child) {
        text-align: right;
        font-variant-numeric: tabular-nums;
    }
    .stats-page .data-table-scroll {
        overflow-x: auto;
        -webkit-overflow-scrolling: touch;
        max-width: 100%;
    }
    @media (max-width: 720px) {
        .stats-page { padding: 1.25rem 1rem; }
        .stats-summary { grid-template-columns: 1fr; }
    }
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
                    Some(s) if s.total_matches == 0 => rsx! {
                        p { class: "empty-state", "No matches tracked for this member yet." }
                    },
                    Some(s) => {
                        let wr = winrate_pct(s.wins, s.total_matches);
                        let mut record = format!("{}W–{}L", s.wins, s.losses);
                        if s.draws > 0 {
                            record.push_str(&format!("–{}D", s.draws));
                        }
                        rsx! {
                            div { class: "stats-summary",
                                div { class: "stat-tile stat-tile-hero",
                                    div { class: "stat-tile-value", "{wr:.1}%" }
                                    div { class: "stat-tile-label", "Win Rate" }
                                    div { class: "stat-tile-record", "{record}" }
                                }
                                div { class: "stat-tile",
                                    div { class: "stat-tile-value", "{s.total_matches}" }
                                    div { class: "stat-tile-label", "Matches" }
                                }
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
                                                let wr_cls = wr_text_class(hero.matches);
                                                let row_cls = if hero.matches < 3 { "stats-row-muted" } else { "" };
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
                                                let wr_cls = wr_text_class(map.matches);
                                                let row_cls = if map.matches < 3 { "stats-row-muted" } else { "" };
                                                rsx! {
                                                    tr { key: "{map.map_name}", class: "{row_cls}",
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
