use dioxus::prelude::*;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::components::{DataTable, SummaryCard, Toast, use_toast};
use crate::hooks::{use_api, use_api_with};
use crate::routes::Route;

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
    #[allow(dead_code)]
    losses: u32,
    #[allow(dead_code)]
    draws: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct PersonalMatch {
    #[allow(dead_code)]
    id: String,
    hero: String,
    map_name: String,
    #[allow(dead_code)]
    game_mode: String,
    role: String,
    outcome: String,
    elims: u32,
    deaths: u32,
    assists: u32,
    damage: u32,
    healing: u32,
    played_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
struct MatchPage {
    data: Vec<PersonalMatch>,
    next_cursor: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum StatsTab {
    Overview,
    Heroes,
    Maps,
    History,
}

const STATS_CSS: &str = r#"
    .stats-page {
        max-width: 1100px;
        margin: 0 auto;
        padding: 2rem 1.5rem;
    }
    .stats-page h1 {
        font-family: var(--font-display);
        font-size: 1.8rem;
        color: var(--text-bright);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        margin-bottom: 0.5rem;
    }
    .stats-header {
        display: flex;
        justify-content: space-between;
        align-items: center;
        flex-wrap: wrap;
        gap: 1rem;
        margin-bottom: 1.5rem;
    }
    .stats-header-actions {
        display: flex;
        gap: 0.5rem;
    }
    .stats-header-actions a, .stats-header-actions button {
        padding: 0.4rem 1rem;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--bg-surface);
        color: var(--text-secondary);
        font-size: 0.8rem;
        cursor: pointer;
        transition: all 0.15s;
        text-decoration: none;
    }
    .stats-header-actions a:hover, .stats-header-actions button:hover {
        color: var(--text-bright);
        border-color: var(--accent-soft);
    }
    .stats-tabs {
        display: flex;
        gap: 0.25rem;
        margin-bottom: 1.5rem;
        border-bottom: 1px solid var(--border);
        padding-bottom: 0;
    }
    .stats-tab {
        padding: 0.6rem 1.2rem;
        border: none;
        background: none;
        color: var(--text-secondary);
        font-family: var(--font-display);
        font-size: 0.9rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.03em;
        cursor: pointer;
        border-bottom: 2px solid transparent;
        margin-bottom: -1px;
        transition: color 0.15s, border-color 0.15s;
    }
    .stats-tab:hover {
        color: var(--text-bright);
    }
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
    .stats-winrate.high { color: #34d399; }
    .stats-winrate.mid { color: #fbbf24; }
    .stats-winrate.low { color: #f87171; }
    .outcome-win { color: #34d399; font-weight: 600; }
    .outcome-loss { color: #f87171; font-weight: 600; }
    .outcome-draw { color: #fbbf24; font-weight: 600; }
    .match-card {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 0.75rem 1rem;
        display: grid;
        grid-template-columns: 80px 1fr 1fr auto;
        gap: 1rem;
        align-items: center;
        font-size: 0.85rem;
        transition: background 0.15s;
    }
    .match-card:hover { background: var(--bg-card-alt); }
    .match-cards { display: flex; flex-direction: column; gap: 0.5rem; }
    .match-card .match-outcome {
        font-family: var(--font-display);
        font-size: 1rem;
        font-weight: 700;
        text-transform: uppercase;
    }
    .match-card .match-hero { color: var(--text-bright); font-weight: 500; }
    .match-card .match-map { color: var(--text-secondary); font-size: 0.8rem; }
    .match-card .match-scoreline { color: var(--text-muted); font-size: 0.8rem; }
    .match-card .match-date { color: var(--text-muted); font-size: 0.75rem; text-align: right; }
    .stats-pagination {
        display: flex;
        justify-content: center;
        gap: 0.75rem;
        margin-top: 1.5rem;
    }
    .stats-pagination button {
        padding: 0.4rem 1rem;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--bg-surface);
        color: var(--text-secondary);
        font-size: 0.8rem;
        cursor: pointer;
        transition: all 0.15s;
    }
    .stats-pagination button:hover:not(:disabled) {
        color: var(--text-bright);
        border-color: var(--accent-soft);
    }
    .stats-pagination button:disabled { opacity: 0.4; cursor: not-allowed; }
    @media (max-width: 640px) {
        .match-card {
            grid-template-columns: 60px 1fr;
            gap: 0.5rem;
        }
        .match-card .match-scoreline,
        .match-card .match-date {
            grid-column: 1 / -1;
        }
    }
"#;

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

fn outcome_class(outcome: &str) -> &'static str {
    match outcome.to_lowercase().as_str() {
        "win" => "outcome-win",
        "loss" => "outcome-loss",
        _ => "outcome-draw",
    }
}

fn format_date(dt: &DateTime<Utc>) -> String {
    dt.format("%b %d, %Y").to_string()
}

#[component]
pub fn Stats() -> Element {
    let stats = use_api::<PersonalStats>("/api/stats/me");
    let heroes = use_api::<Vec<HeroStats>>("/api/stats/me/heroes");
    let maps = use_api::<Vec<MapStats>>("/api/stats/me/maps");

    let mut tab = use_signal(|| StatsTab::Overview);

    let mut page_cursor = use_signal(|| Option::<String>::None);
    let mut cursor_history: Signal<Vec<Option<String>>> = use_signal(|| vec![None]);

    let matches = use_api_with::<MatchPage>(move || {
        let cursor = page_cursor();
        match cursor {
            Some(c) => format!("/api/stats/me/matches?limit=25&cursor={c}"),
            None => "/api/stats/me/matches?limit=25".to_string(),
        }
    });

    rsx! {
        style { {STATS_CSS} }
        style { {crate::styles::admin::CSS} }

        div { class: "stats-page",
            div { class: "stats-header",
                h1 { "My Stats" }
                div { class: "stats-header-actions",
                    Link { to: Route::StatsTokens {}, "Daemon Tokens" }
                }
            }

            // Summary cards (always visible)
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

            // Tabs
            div { class: "stats-tabs",
                button {
                    class: if tab() == StatsTab::Overview { "stats-tab active" } else { "stats-tab" },
                    onclick: move |_| tab.set(StatsTab::Overview),
                    "Overview"
                }
                button {
                    class: if tab() == StatsTab::Heroes { "stats-tab active" } else { "stats-tab" },
                    onclick: move |_| tab.set(StatsTab::Heroes),
                    "Heroes"
                }
                button {
                    class: if tab() == StatsTab::Maps { "stats-tab active" } else { "stats-tab" },
                    onclick: move |_| tab.set(StatsTab::Maps),
                    "Maps"
                }
                button {
                    class: if tab() == StatsTab::History { "stats-tab active" } else { "stats-tab" },
                    onclick: move |_| {
                        tab.set(StatsTab::History);
                        page_cursor.set(None);
                        cursor_history.set(vec![None]);
                    },
                    "History"
                }
            }

            // Tab content
            match tab() {
                StatsTab::Overview => rsx! {
                    p { class: "empty-state",
                        style: "padding: 1rem 0; text-align: left;",
                        "Select a tab above to view detailed hero stats, map stats, or match history."
                    }
                },
                StatsTab::Heroes => rsx! {
                    {
                        let data = heroes.data.read();
                        let list = data.as_ref().and_then(|d| d.as_ref());
                        match list {
                            None => rsx! { p { class: "loading-state", "Loading hero stats..." } },
                            Some(list) if list.is_empty() => rsx! {
                                p { class: "empty-state", "No hero stats recorded yet." }
                            },
                            Some(list) => {
                                let mut sorted: Vec<_> = list.iter().collect();
                                sorted.sort_by(|a, b| b.matches.cmp(&a.matches));
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
                StatsTab::Maps => rsx! {
                    {
                        let data = maps.data.read();
                        let list = data.as_ref().and_then(|d| d.as_ref());
                        match list {
                            None => rsx! { p { class: "loading-state", "Loading map stats..." } },
                            Some(list) if list.is_empty() => rsx! {
                                p { class: "empty-state", "No map stats recorded yet." }
                            },
                            Some(list) => {
                                let mut sorted: Vec<_> = list.iter().collect();
                                sorted.sort_by(|a, b| b.matches.cmp(&a.matches));
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
                StatsTab::History => rsx! {
                    {
                        let data = matches.data.read();
                        let page = data.as_ref().and_then(|d| d.as_ref());
                        match page {
                            None => rsx! { p { class: "loading-state", "Loading match history..." } },
                            Some(page) if page.data.is_empty() => rsx! {
                                p { class: "empty-state", "No matches recorded yet." }
                            },
                            Some(page) => {
                                let has_next = page.next_cursor.is_some();
                                let next_c = page.next_cursor.clone();
                                let can_prev = cursor_history().len() > 1;
                                rsx! {
                                    div { class: "match-cards",
                                        for m in page.data.iter() {
                                            {
                                                let oc = outcome_class(&m.outcome);
                                                let date = format_date(&m.played_at);
                                                rsx! {
                                                    div { class: "match-card", key: "{m.id}",
                                                        div { class: "match-outcome {oc}", "{m.outcome}" }
                                                        div {
                                                            div { class: "match-hero", "{m.hero}" }
                                                            div { class: "match-map", "{m.map_name} · {m.role}" }
                                                        }
                                                        div { class: "match-scoreline",
                                                            "{m.elims}E / {m.deaths}D / {m.assists}A · {m.damage} dmg · {m.healing} heal"
                                                        }
                                                        div { class: "match-date", "{date}" }
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
                            },
                        }
                    }
                },
            }
        }
    }
}
