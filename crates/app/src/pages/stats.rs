use dioxus::prelude::*;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::components::charts::{BarEntry, DonutChart, DonutSegment, HBarChart};
use crate::components::{DataTable, SummaryCard};
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
    losses: u32,
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

// -- Role aggregation --

struct RoleAgg {
    name: &'static str,
    color: &'static str,
    matches: u32,
    wins: u32,
    weighted_elims: f64,
    weighted_deaths: f64,
    weighted_damage: f64,
    weighted_healing: f64,
}

impl RoleAgg {
    fn new(name: &'static str, color: &'static str) -> Self {
        Self {
            name,
            color,
            matches: 0,
            wins: 0,
            weighted_elims: 0.0,
            weighted_deaths: 0.0,
            weighted_damage: 0.0,
            weighted_healing: 0.0,
        }
    }

    fn winrate(&self) -> f64 {
        winrate_pct(self.wins, self.matches)
    }

    fn avg_elims(&self) -> f64 {
        if self.matches == 0 {
            0.0
        } else {
            self.weighted_elims / self.matches as f64
        }
    }

    fn avg_deaths(&self) -> f64 {
        if self.matches == 0 {
            0.0
        } else {
            self.weighted_deaths / self.matches as f64
        }
    }

    fn avg_damage(&self) -> f64 {
        if self.matches == 0 {
            0.0
        } else {
            self.weighted_damage / self.matches as f64
        }
    }

    fn avg_healing(&self) -> f64 {
        if self.matches == 0 {
            0.0
        } else {
            self.weighted_healing / self.matches as f64
        }
    }
}

fn hero_to_role(name: &str) -> &'static str {
    match name {
        "D.Va" | "Domina" | "Doomfist" | "Hazard" | "Junker Queen" | "Mauga" | "Orisa"
        | "Ramattra" | "Reinhardt" | "Roadhog" | "Sigma" | "Winston" | "Wrecking Ball"
        | "Zarya" => "Tank",
        "Ana" | "Baptiste" | "Brigitte" | "Illari" | "Juno" | "Kiriko" | "Lifeweaver" | "Lúcio"
        | "Mercy" | "Mizuki" | "Moira" | "Wuyang" | "Zenyatta" => "Support",
        _ => "Damage",
    }
}

fn aggregate_roles(heroes: &[HeroStats]) -> Vec<RoleAgg> {
    let mut tank = RoleAgg::new("Tank", "#3b82f6");
    let mut damage = RoleAgg::new("Damage", "#ef4444");
    let mut support = RoleAgg::new("Support", "#22c55e");

    for h in heroes {
        let agg = match hero_to_role(&h.hero) {
            "Tank" => &mut tank,
            "Support" => &mut support,
            _ => &mut damage,
        };
        agg.matches += h.matches;
        agg.wins += h.wins;
        agg.weighted_elims += h.avg_elims * h.matches as f64;
        agg.weighted_deaths += h.avg_deaths * h.matches as f64;
        agg.weighted_damage += h.avg_damage * h.matches as f64;
        agg.weighted_healing += h.avg_healing * h.matches as f64;
    }

    vec![tank, damage, support]
}

// -- Map grouping --

fn map_game_mode(name: &str) -> &'static str {
    match name {
        "Circuit Royal"
        | "Dorado"
        | "Havana"
        | "Junkertown"
        | "Rialto"
        | "Route 66"
        | "Shambali Monastery"
        | "Watchpoint: Gibraltar" => "Escort",
        "Blizzard World" | "Eichenwalde" | "Hollywood" | "King's Row" | "Midtown" | "Numbani"
        | "Paraíso" => "Hybrid",
        "Antarctic Peninsula"
        | "Busan"
        | "Ilios"
        | "Lijiang Tower"
        | "Nepal"
        | "Oasis"
        | "Samoa" => "Control",
        "Colosseo" | "Esperança" | "New Queen Street" | "Runasapi" => "Push",
        "Aatlis" | "New Junk City" | "Suravasa" => "Flashpoint",
        "Hanaoka" | "Throne of Anubis" => "Clash",
        _ => "Other",
    }
}

const MODE_ORDER: &[&str] = &[
    "Escort",
    "Hybrid",
    "Control",
    "Push",
    "Flashpoint",
    "Clash",
    "Other",
];

fn mode_color(mode: &str) -> &'static str {
    match mode {
        "Escort" => "#60a5fa",
        "Hybrid" => "#a78bfa",
        "Control" => "#34d399",
        "Push" => "#fbbf24",
        "Flashpoint" => "#fb923c",
        "Clash" => "#f87171",
        _ => "#94a3b8",
    }
}

// -- Helpers --

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

    /* Overview: role layout */
    .overview-grid {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 1.5rem;
        margin-top: 1rem;
    }
    .overview-section {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.25rem;
    }
    .overview-section h3 {
        font-family: var(--font-display);
        font-size: 0.85rem;
        color: var(--text-muted);
        text-transform: uppercase;
        letter-spacing: 0.06em;
        margin: 0 0 1rem;
    }
    .role-cards { display: flex; flex-direction: column; gap: 0.75rem; }
    .role-card {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        padding: 0.6rem 0.75rem;
        border-radius: 8px;
        background: var(--bg-surface);
        border-left: 3px solid transparent;
    }
    .role-card-info { flex: 1; }
    .role-card-name {
        font-weight: 600;
        font-size: 0.85rem;
        color: var(--text-bright);
    }
    .role-card-sub {
        font-size: 0.75rem;
        color: var(--text-muted);
        margin-top: 0.15rem;
    }
    .role-card-wr {
        font-family: var(--font-display);
        font-size: 1.1rem;
        font-weight: 700;
    }
    .role-card-wr.high { color: #34d399; }
    .role-card-wr.mid { color: #fbbf24; }
    .role-card-wr.low { color: #f87171; }

    /* Heroes: chart + table */
    .heroes-chart-section {
        margin-bottom: 1.5rem;
    }
    .section-title {
        font-family: var(--font-display);
        font-size: 0.8rem;
        color: var(--text-muted);
        text-transform: uppercase;
        letter-spacing: 0.06em;
        margin-bottom: 0.75rem;
    }

    /* Maps: game mode groups */
    .map-mode-group { margin-bottom: 1.5rem; }
    .map-mode-header {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        margin-bottom: 0.6rem;
        padding-bottom: 0.4rem;
        border-bottom: 1px solid var(--border);
    }
    .map-mode-name {
        font-family: var(--font-display);
        font-size: 0.85rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .map-mode-agg {
        font-size: 0.75rem;
        color: var(--text-muted);
        margin-left: auto;
    }

    @media (max-width: 768px) {
        .overview-grid { grid-template-columns: 1fr; }
    }
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

fn wr_value_class(pct: f64) -> &'static str {
    if pct >= 55.0 {
        "role-card-wr high"
    } else if pct >= 45.0 {
        "role-card-wr mid"
    } else {
        "role-card-wr low"
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

fn wr_bar_color(pct: f64) -> &'static str {
    if pct >= 55.0 {
        "#34d399"
    } else if pct >= 45.0 {
        "#fbbf24"
    } else {
        "#f87171"
    }
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
                    {
                        let data = heroes.data.read();
                        let list = data.as_ref().and_then(|d| d.as_ref());
                        match list {
                            None => rsx! { p { class: "loading-state", "Loading role breakdown..." } },
                            Some(list) if list.is_empty() => rsx! {
                                p { class: "empty-state", "Play some matches to see your role breakdown." }
                            },
                            Some(list) => {
                                let roles = aggregate_roles(list);
                                let total_matches: u32 = roles.iter().map(|r| r.matches).sum();
                                let donut_segments: Vec<DonutSegment> = roles.iter().map(|r| DonutSegment {
                                    label: r.name.to_string(),
                                    value: r.matches as f64,
                                    color: r.color.to_string(),
                                }).collect();

                                rsx! {
                                    div { class: "overview-grid",
                                        div { class: "overview-section",
                                            h3 { "Role Distribution" }
                                            DonutChart {
                                                segments: donut_segments,
                                                center_value: total_matches.to_string(),
                                                center_label: "matches".to_string(),
                                            }
                                        }
                                        div { class: "overview-section",
                                            h3 { "Role Performance" }
                                            div { class: "role-cards",
                                                {roles.iter().filter(|r| r.matches > 0).map(|r| {
                                                    let wr = r.winrate();
                                                    let wr_cls = wr_value_class(wr);
                                                    let border_color = r.color;
                                                    let name = r.name;
                                                    let avg_e = r.avg_elims();
                                                    let avg_d = r.avg_deaths();
                                                    let avg_dmg = r.avg_damage();
                                                    let avg_heal = r.avg_healing();
                                                    let m = r.matches;
                                                    rsx! {
                                                        div {
                                                            class: "role-card",
                                                            key: "{name}",
                                                            style: "border-left-color:{border_color};",
                                                            div { class: "role-card-info",
                                                                div { class: "role-card-name", "{name}" }
                                                                div { class: "role-card-sub",
                                                                    "{m} matches · {avg_e:.1}E {avg_d:.1}D · {avg_dmg:.0}dmg {avg_heal:.0}heal"
                                                                }
                                                            }
                                                            div { class: "{wr_cls}", "{wr:.1}%" }
                                                        }
                                                    }
                                                })}
                                            }
                                        }
                                    }
                                }
                            },
                        }
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
                                sorted.sort_by_key(|b| std::cmp::Reverse(b.matches));

                                // Top heroes by win rate (min 3 matches, top 8)
                                let mut by_wr: Vec<_> = list.iter()
                                    .filter(|h| h.matches >= 3)
                                    .collect();
                                by_wr.sort_by(|a, b| {
                                    let wa = winrate_pct(a.wins, a.matches);
                                    let wb = winrate_pct(b.wins, b.matches);
                                    wb.partial_cmp(&wa).unwrap_or(std::cmp::Ordering::Equal)
                                });
                                let top_heroes: Vec<BarEntry> = by_wr.iter().take(8).map(|h| {
                                    let wr = winrate_pct(h.wins, h.matches);
                                    BarEntry {
                                        label: h.hero.clone(),
                                        value: wr,
                                        color: wr_bar_color(wr).to_string(),
                                        display: format!("{wr:.1}%"),
                                    }
                                }).collect();

                                rsx! {
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
                                // Group by game mode
                                let mut groups: std::collections::HashMap<&str, Vec<&MapStats>> =
                                    std::collections::HashMap::new();
                                for m in list.iter() {
                                    let mode = map_game_mode(&m.map_name);
                                    groups.entry(mode).or_default().push(m);
                                }

                                // Sort modes by MODE_ORDER
                                let ordered_modes: Vec<&&str> = MODE_ORDER.iter()
                                    .filter(|mode| groups.contains_key(**mode))
                                    .collect();

                                rsx! {
                                    {ordered_modes.iter().map(|&&mode| {
                                        let maps_in_mode = &groups[mode];
                                        let total_m: u32 = maps_in_mode.iter().map(|m| m.matches).sum();
                                        let total_w: u32 = maps_in_mode.iter().map(|m| m.wins).sum();
                                        let mode_wr = winrate_pct(total_w, total_m);
                                        let mode_wr_cls = winrate_class(mode_wr);
                                        let mc = mode_color(mode);

                                        let mut sorted_maps: Vec<&&MapStats> = maps_in_mode.iter().collect();
                                        sorted_maps.sort_by_key(|b| std::cmp::Reverse(b.matches));

                                        let bar_entries: Vec<BarEntry> = sorted_maps.iter().map(|m| {
                                            let wr = winrate_pct(m.wins, m.matches);
                                            BarEntry {
                                                label: m.map_name.clone(),
                                                value: wr,
                                                color: mc.to_string(),
                                                display: format!("{wr:.1}% ({} games)", m.matches),
                                            }
                                        }).collect();

                                        rsx! {
                                            div { class: "map-mode-group", key: "{mode}",
                                                div { class: "map-mode-header",
                                                    span {
                                                        class: "map-mode-name",
                                                        style: "color:{mc};",
                                                        "{mode}"
                                                    }
                                                    span { class: "map-mode-agg",
                                                        "{total_m} matches · "
                                                        span { class: "{mode_wr_cls}", "{mode_wr:.1}%" }
                                                    }
                                                }
                                                HBarChart { entries: bar_entries }
                                            }
                                        }
                                    })}
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
