use dioxus::prelude::*;

use stat_tracker::config::Config;
use stat_tracker::stats::{HeroMapBreakdown, compute_stats};

use super::live_data;

#[component]
pub fn StatsPanel() -> Element {
    let config = use_signal(|| Config::load().unwrap_or_default());
    let mut refresh_tick = use_signal(|| 0u32);
    let mut db_locked = use_signal(|| false);

    let matches = use_resource(move || {
        let data_dir = config().data_dir.clone();
        let _tick = refresh_tick();
        async move {
            let live = live_data::fetch_live_matches(&data_dir).await;
            db_locked.set(live.db_locked);
            // Stats are per GAME: collapse the capture snapshots of each
            // session to its final scoreboard.
            stat_tracker::storage::latest_per_game(live.matches)
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
                    p { class: "text-dim text-sm", "Live view from daemon snapshot (daemon is running)" }
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
                                {
                                    let undecided = stats.overall.total
                                        - stats.overall.wins
                                        - stats.overall.losses
                                        - stats.overall.draws;
                                    rsx! {
                                        if undecided > 0 {
                                            div { class: "stat-block",
                                                div { class: "stat-big text-dim", "{undecided}" }
                                                div { class: "stat-label", "Unknown" }
                                            }
                                        }
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
    let line_color = if last_wr >= 50.0 {
        "#22c55e"
    } else {
        "#ef4444"
    };

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
