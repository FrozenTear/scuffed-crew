use std::collections::HashMap;

use dioxus::prelude::*;

use stat_tracker::config::Config;
use stat_tracker::storage::{LocalStore, PersonalMatch};

struct OverallStats {
    total: usize,
    wins: usize,
    losses: usize,
    draws: usize,
}

impl OverallStats {
    fn win_rate(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.wins as f64 / self.total as f64) * 100.0
    }
}

struct HeroStats {
    hero: String,
    role: String,
    games: usize,
    wins: usize,
    avg_elims: f64,
    avg_deaths: f64,
    avg_assists: f64,
    avg_damage: f64,
    avg_healing: f64,
    avg_mitigation: f64,
}

impl HeroStats {
    fn win_rate(&self) -> f64 {
        if self.games == 0 {
            return 0.0;
        }
        (self.wins as f64 / self.games as f64) * 100.0
    }
}

struct RoleStats {
    role: String,
    games: usize,
    wins: usize,
}

impl RoleStats {
    fn win_rate(&self) -> f64 {
        if self.games == 0 {
            return 0.0;
        }
        (self.wins as f64 / self.games as f64) * 100.0
    }
}

struct MapStats {
    map_name: String,
    games: usize,
    wins: usize,
    losses: usize,
}

impl MapStats {
    fn win_rate(&self) -> f64 {
        if self.games == 0 {
            return 0.0;
        }
        (self.wins as f64 / self.games as f64) * 100.0
    }
}

#[derive(Clone, PartialEq)]
struct HeroMapBreakdown {
    map_name: String,
    games: usize,
    wins: usize,
}

impl HeroMapBreakdown {
    fn win_rate(&self) -> f64 {
        if self.games == 0 {
            return 0.0;
        }
        (self.wins as f64 / self.games as f64) * 100.0
    }
}

struct ComputedStats {
    overall: OverallStats,
    heroes: Vec<HeroStats>,
    roles: Vec<RoleStats>,
    maps: Vec<MapStats>,
    hero_maps: HashMap<String, Vec<HeroMapBreakdown>>,
    rolling_wr: Vec<f64>,
}

fn compute_stats(matches: &[PersonalMatch]) -> ComputedStats {
    let mut wins = 0usize;
    let mut losses = 0usize;
    let mut draws = 0usize;

    struct Acc {
        role: String,
        games: usize,
        wins: usize,
        elims: u64,
        deaths: u64,
        assists: u64,
        damage: u64,
        healing: u64,
        mitigation: u64,
    }

    let mut hero_acc: HashMap<String, Acc> = HashMap::new();
    let mut role_map: HashMap<String, (usize, usize)> = HashMap::new();
    let mut map_acc: HashMap<String, (usize, usize, usize)> = HashMap::new();
    let mut hero_map_acc: HashMap<String, HashMap<String, (usize, usize)>> = HashMap::new();

    for m in matches {
        let is_win = m.outcome.eq_ignore_ascii_case("win");
        let is_loss = m.outcome.eq_ignore_ascii_case("loss");
        if is_win {
            wins += 1;
        } else if is_loss {
            losses += 1;
        } else {
            draws += 1;
        }

        let entry = hero_acc.entry(m.hero.clone()).or_insert_with(|| Acc {
            role: m.role.clone(),
            games: 0,
            wins: 0,
            elims: 0,
            deaths: 0,
            assists: 0,
            damage: 0,
            healing: 0,
            mitigation: 0,
        });
        entry.games += 1;
        if is_win {
            entry.wins += 1;
        }
        entry.elims += m.elims as u64;
        entry.deaths += m.deaths as u64;
        entry.assists += m.assists as u64;
        entry.damage += m.damage as u64;
        entry.healing += m.healing as u64;
        entry.mitigation += m.mitigation as u64;

        let role_entry = role_map.entry(m.role.clone()).or_insert((0, 0));
        role_entry.0 += 1;
        if is_win {
            role_entry.1 += 1;
        }

        if !m.map_name.is_empty() {
            let me = map_acc.entry(m.map_name.clone()).or_insert((0, 0, 0));
            me.0 += 1;
            if is_win {
                me.1 += 1;
            }
            if is_loss {
                me.2 += 1;
            }

            let hm = hero_map_acc
                .entry(m.hero.clone())
                .or_default()
                .entry(m.map_name.clone())
                .or_insert((0, 0));
            hm.0 += 1;
            if is_win {
                hm.1 += 1;
            }
        }
    }

    let overall = OverallStats {
        total: matches.len(),
        wins,
        losses,
        draws,
    };

    let mut heroes: Vec<HeroStats> = hero_acc
        .into_iter()
        .map(|(hero, a)| {
            let g = a.games as f64;
            HeroStats {
                hero,
                role: a.role,
                games: a.games,
                wins: a.wins,
                avg_elims: a.elims as f64 / g,
                avg_deaths: a.deaths as f64 / g,
                avg_assists: a.assists as f64 / g,
                avg_damage: a.damage as f64 / g,
                avg_healing: a.healing as f64 / g,
                avg_mitigation: a.mitigation as f64 / g,
            }
        })
        .collect();
    heroes.sort_by(|a, b| b.games.cmp(&a.games));

    let role_order = ["Tank", "Damage", "Support"];
    let mut roles: Vec<RoleStats> = role_map
        .into_iter()
        .map(|(role, (games, wins))| RoleStats { role, games, wins })
        .collect();
    roles.sort_by_key(|r| {
        role_order
            .iter()
            .position(|o| o.eq_ignore_ascii_case(&r.role))
            .unwrap_or(99)
    });

    let mut maps: Vec<MapStats> = map_acc
        .into_iter()
        .map(|(map_name, (games, wins, losses))| MapStats {
            map_name,
            games,
            wins,
            losses,
        })
        .collect();
    maps.sort_by(|a, b| b.games.cmp(&a.games));

    let mut hero_maps: HashMap<String, Vec<HeroMapBreakdown>> = HashMap::new();
    for (hero, map_data) in hero_map_acc {
        let mut breakdowns: Vec<HeroMapBreakdown> = map_data
            .into_iter()
            .map(|(map_name, (games, wins))| HeroMapBreakdown {
                map_name,
                games,
                wins,
            })
            .collect();
        breakdowns.sort_by(|a, b| b.games.cmp(&a.games));
        hero_maps.insert(hero, breakdowns);
    }

    // Rolling 10-game winrate (matches come in DESC order, reverse for chronological)
    let mut rolling_wr = Vec::new();
    let window = 10usize;
    let chronological: Vec<&PersonalMatch> = matches.iter().rev().collect();
    let mut win_count = 0usize;
    for (i, m) in chronological.iter().enumerate() {
        if m.outcome.eq_ignore_ascii_case("win") {
            win_count += 1;
        }
        if i >= window {
            if chronological[i - window]
                .outcome
                .eq_ignore_ascii_case("win")
            {
                win_count -= 1;
            }
        }
        let denom = (i + 1).min(window);
        rolling_wr.push((win_count as f64 / denom as f64) * 100.0);
    }

    ComputedStats {
        overall,
        heroes,
        roles,
        maps,
        hero_maps,
        rolling_wr,
    }
}

#[component]
pub fn StatsPanel() -> Element {
    let config = use_signal(|| Config::load().unwrap_or_default());
    let mut refresh_tick = use_signal(|| 0u32);
    let mut db_locked = use_signal(|| false);

    let matches = use_resource(move || {
        let data_dir = config().data_dir.clone();
        let _tick = refresh_tick();
        async move {
            match LocalStore::open(&data_dir).await {
                Ok(store) => {
                    db_locked.set(false);
                    store.get_all_matches().await.unwrap_or_default()
                }
                Err(_) => {
                    db_locked.set(true);
                    stat_tracker::storage::read_match_log(&data_dir)
                }
            }
        }
    });

    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            refresh_tick += 1;
        }
    });

    let db_locked = db_locked();

    rsx! {
        div { class: "panel panel-wide",
            h2 { "Stats" }

            if db_locked {
                div { class: "card card-info",
                    p { class: "text-dim text-sm", "Reading from log file (daemon is running)" }
                }
            }

            match &*matches.read() {
                Some(m) if m.is_empty() => rsx! {
                    div { class: "card",
                        p { class: "text-dim", "No matches recorded yet." }
                    }
                },
                Some(m) => {
                    let stats = compute_stats(m);
                    rsx! {
                        div { class: "card",
                            h3 { "Overview" }
                            div { class: "stats-grid",
                                div { class: "stat-block",
                                    div { class: "stat-big", "{stats.overall.total}" }
                                    div { class: "stat-label", "Games" }
                                }
                                div { class: "stat-block",
                                    div { class: "stat-big stat-win", "{stats.overall.wins}" }
                                    div { class: "stat-label", "Wins" }
                                }
                                div { class: "stat-block",
                                    div { class: "stat-big stat-loss", "{stats.overall.losses}" }
                                    div { class: "stat-label", "Losses" }
                                }
                                if stats.overall.draws > 0 {
                                    div { class: "stat-block",
                                        div { class: "stat-big", "{stats.overall.draws}" }
                                        div { class: "stat-label", "Draws" }
                                    }
                                }
                                div { class: "stat-block",
                                    div { class: "stat-big", "{stats.overall.win_rate():.1}%" }
                                    div { class: "stat-label", "Win Rate" }
                                }
                            }
                        }

                        if stats.rolling_wr.len() >= 2 {
                            WinTrendChart { rolling_wr: stats.rolling_wr.clone() }
                        }

                        div { class: "card",
                            h3 { "By Role" }
                            div { class: "role-grid",
                                for r in stats.roles.iter() {
                                    div { class: "role-card",
                                        div { class: "role-name role-{r.role.to_lowercase()}", "{r.role}" }
                                        div { class: "role-stats",
                                            span { "{r.games} games" }
                                            span { class: "text-dim", " \u{00b7} " }
                                            span { "{r.win_rate():.0}% WR" }
                                        }
                                        div { class: "wr-bar",
                                            div {
                                                class: "wr-fill",
                                                style: "width: {r.win_rate():.1}%",
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if !stats.maps.is_empty() {
                            div { class: "card",
                                h3 { "By Map" }
                                div { class: "map-table",
                                    div { class: "map-header",
                                        span { class: "col-map-name", "Map" }
                                        span { class: "col-map-games", "Games" }
                                        span { class: "col-map-wl", "W/L" }
                                        span { class: "col-map-wr", "Win %" }
                                        span { class: "col-map-bar", "" }
                                    }
                                    for ms in stats.maps.iter() {
                                        div { class: "map-row",
                                            span { class: "col-map-name", "{ms.map_name}" }
                                            span { class: "col-map-games", "{ms.games}" }
                                            span { class: "col-map-wl", "{ms.wins}/{ms.losses}" }
                                            {
                                                let wr = ms.win_rate();
                                                let wr_class = if wr >= 55.0 { "stat-win" } else if wr <= 45.0 { "stat-loss" } else { "" };
                                                rsx! {
                                                    span { class: "col-map-wr {wr_class}", "{wr:.0}%" }
                                                }
                                            }
                                            span { class: "col-map-bar",
                                                div { class: "wr-bar",
                                                    div {
                                                        class: "wr-fill",
                                                        style: "width: {ms.win_rate():.1}%",
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            MapBarChart { maps: stats.maps.iter().map(|ms| (ms.map_name.clone(), ms.wins, ms.losses, ms.games.saturating_sub(ms.wins).saturating_sub(ms.losses))).collect() }
                        }

                        div { class: "card",
                            h3 { "By Hero" }
                            div { class: "hero-table",
                                div { class: "hero-header",
                                    span { class: "col-hero-name", "Hero" }
                                    span { class: "col-hero-role", "Role" }
                                    span { class: "col-hero-games", "Games" }
                                    span { class: "col-hero-wr", "Win %" }
                                    span { class: "col-stat", "E" }
                                    span { class: "col-stat", "D" }
                                    span { class: "col-stat", "A" }
                                    span { class: "col-stat", "DMG" }
                                    span { class: "col-stat", "HLG" }
                                    span { class: "col-stat", "MIT" }
                                }
                                for h in stats.heroes.iter() {
                                    {
                                        let map_breakdown = stats.hero_maps.get(&h.hero).cloned().unwrap_or_default();
                                        let has_maps = map_breakdown.len() >= 2 && h.games >= 3;
                                        rsx! {
                                            HeroRowComponent {
                                                hero: h.hero.clone(),
                                                role: h.role.clone(),
                                                games: h.games,
                                                win_rate: h.win_rate(),
                                                avg_elims: h.avg_elims,
                                                avg_deaths: h.avg_deaths,
                                                avg_assists: h.avg_assists,
                                                avg_damage: h.avg_damage,
                                                avg_healing: h.avg_healing,
                                                avg_mitigation: h.avg_mitigation,
                                                map_breakdown: map_breakdown,
                                                expandable: has_maps,
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
                None => rsx! {
                    div { class: "card",
                        p { class: "text-dim", "Loading..." }
                    }
                },
            }
        }
    }
}

#[derive(Clone, PartialEq, Props)]
struct HeroRowProps {
    hero: String,
    role: String,
    games: usize,
    win_rate: f64,
    avg_elims: f64,
    avg_deaths: f64,
    avg_assists: f64,
    avg_damage: f64,
    avg_healing: f64,
    avg_mitigation: f64,
    map_breakdown: Vec<HeroMapBreakdown>,
    expandable: bool,
}

#[allow(non_snake_case)]
fn HeroRowComponent(props: HeroRowProps) -> Element {
    let mut expanded = use_signal(|| false);
    let role_class = format!("role-{}", props.role.to_lowercase());
    let wr = props.win_rate;
    let wr_class = if wr >= 55.0 {
        "stat-win"
    } else if wr <= 45.0 {
        "stat-loss"
    } else {
        ""
    };

    let row_class = if props.expandable {
        "hero-row hero-row-expandable"
    } else {
        "hero-row"
    };

    let arrow = if props.expandable {
        if expanded() { "\u{25BC} " } else { "\u{25B6} " }
    } else {
        ""
    };

    rsx! {
        div {
            class: "{row_class}",
            onclick: move |_| {
                if props.expandable {
                    expanded.set(!expanded());
                }
            },
            span { class: "col-hero-name", "{arrow}{props.hero}" }
            span { class: "col-hero-role {role_class}", "{props.role}" }
            span { class: "col-hero-games", "{props.games}" }
            span { class: "col-hero-wr {wr_class}", "{wr:.0}%" }
            span { class: "col-stat", "{props.avg_elims:.1}" }
            span { class: "col-stat", "{props.avg_deaths:.1}" }
            span { class: "col-stat", "{props.avg_assists:.1}" }
            span { class: "col-stat", "{props.avg_damage:.0}" }
            span { class: "col-stat", "{props.avg_healing:.0}" }
            span { class: "col-stat", "{props.avg_mitigation:.0}" }
        }
        if expanded() {
            for mb in props.map_breakdown.iter() {
                {
                    let mwr = mb.win_rate();
                    let mwr_class = if mwr >= 55.0 { "stat-win" } else if mwr <= 45.0 { "stat-loss" } else { "" };
                    rsx! {
                        div { class: "hero-row hero-row-sub",
                            span { class: "col-hero-name col-sub-indent", "{mb.map_name}" }
                            span { class: "col-hero-role", "" }
                            span { class: "col-hero-games", "{mb.games}" }
                            span { class: "col-hero-wr {mwr_class}", "{mwr:.0}%" }
                            span { class: "col-stat", "" }
                            span { class: "col-stat", "" }
                            span { class: "col-stat", "" }
                            span { class: "col-stat", "" }
                            span { class: "col-stat", "" }
                            span { class: "col-stat", "" }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, PartialEq, Props)]
struct WinTrendChartProps {
    rolling_wr: Vec<f64>,
}

#[allow(non_snake_case)]
fn WinTrendChart(props: WinTrendChartProps) -> Element {
    let values = &props.rolling_wr;
    let width = 400.0_f64;
    let height = 80.0_f64;
    let padding = 8.0_f64;
    let usable_w = width - padding * 2.0;
    let usable_h = height - padding * 2.0;

    let min_val = 0.0_f64;
    let max_val = 100.0_f64;
    let range = max_val - min_val;

    let points: Vec<String> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = if values.len() > 1 {
                padding + (i as f64 / (values.len() - 1) as f64) * usable_w
            } else {
                width / 2.0
            };
            let y = padding + usable_h - ((v - min_val) / range) * usable_h;
            format!("{x:.1},{y:.1}")
        })
        .collect();

    let polyline_points = points.join(" ");
    let last_wr = values.last().copied().unwrap_or(50.0);
    let line_color = if last_wr >= 50.0 { "#22c55e" } else { "#ef4444" };

    let fifty_y = padding + usable_h - ((50.0 - min_val) / range) * usable_h;

    let svg_content = format!(
        r##"<svg viewBox="0 0 {width} {height}" xmlns="http://www.w3.org/2000/svg" class="trend-svg">
            <line x1="{padding}" y1="{fifty_y}" x2="{}" y2="{fifty_y}" stroke="#2a2a3a" stroke-width="1" stroke-dasharray="4,4"/>
            <polyline points="{polyline_points}" fill="none" stroke="{line_color}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>"##,
        width - padding
    );

    rsx! {
        div { class: "card",
            h3 { "Win Rate Trend" }
            div { class: "trend-header",
                span { class: "trend-current",
                    span {
                        class: if last_wr >= 50.0 { "stat-win" } else { "stat-loss" },
                        "{last_wr:.1}%"
                    }
                    span { class: "text-dim", " current" }
                }
                span { class: "text-dim text-sm", "Rolling 10-game window" }
            }
            div { dangerous_inner_html: "{svg_content}" }
        }
    }
}

#[derive(Clone, PartialEq, Props)]
struct MapBarChartProps {
    maps: Vec<(String, usize, usize, usize)>,
}

#[allow(non_snake_case)]
fn MapBarChart(props: MapBarChartProps) -> Element {
    if props.maps.is_empty() {
        return rsx! {};
    }

    let max_games = props.maps.iter().map(|m| m.1 + m.2 + m.3).max().unwrap_or(1);
    let bar_height = 24.0_f64;
    let gap = 4.0_f64;
    let label_width = 120.0_f64;
    let chart_width = 300.0_f64;
    let total_width = label_width + chart_width + 50.0;
    let total_height = props.maps.len() as f64 * (bar_height + gap) + gap;

    let mut bars = String::new();
    for (i, (name, wins, losses, draws)) in props.maps.iter().enumerate() {
        let y = gap + i as f64 * (bar_height + gap);
        let total = wins + losses + draws;
        let w_frac = *wins as f64 / max_games as f64;
        let l_frac = *losses as f64 / max_games as f64;
        let d_frac = *draws as f64 / max_games as f64;

        let w_width = w_frac * chart_width;
        let l_width = l_frac * chart_width;
        let d_width = d_frac * chart_width;

        bars.push_str(&format!(
            r##"<text x="{}" y="{}" fill="#8888a0" font-size="11" font-family="Inter, sans-serif" text-anchor="end" dominant-baseline="middle">{}</text>"##,
            label_width - 8.0,
            y + bar_height / 2.0,
            name
        ));

        let mut x = label_width;
        if *wins > 0 {
            bars.push_str(&format!(
                r##"<rect x="{x}" y="{y}" width="{w_width}" height="{bar_height}" rx="3" fill="#22c55e"/>"##
            ));
            x += w_width;
        }
        if *losses > 0 {
            bars.push_str(&format!(
                r##"<rect x="{x}" y="{y}" width="{l_width}" height="{bar_height}" rx="3" fill="#ef4444"/>"##
            ));
            x += l_width;
        }
        if *draws > 0 {
            bars.push_str(&format!(
                r##"<rect x="{x}" y="{y}" width="{d_width}" height="{bar_height}" rx="3" fill="#8888a0"/>"##
            ));
        }

        bars.push_str(&format!(
            r##"<text x="{}" y="{}" fill="#8888a0" font-size="10" font-family="JetBrains Mono, monospace" dominant-baseline="middle">{}</text>"##,
            label_width + (w_width + l_width + d_width) + 6.0,
            y + bar_height / 2.0,
            total
        ));
    }

    let svg_content = format!(
        r##"<svg viewBox="0 0 {total_width} {total_height}" xmlns="http://www.w3.org/2000/svg" class="map-bar-svg">{bars}</svg>"##
    );

    rsx! {
        div { class: "card",
            h3 { "Map Win/Loss" }
            div { dangerous_inner_html: "{svg_content}" }
        }
    }
}
