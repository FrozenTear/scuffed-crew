use dioxus::prelude::*;

use crate::components::charts::{BarEntry, HBarChart};
use crate::hooks::ApiResource;

use super::{MapStats, load_error_state, map_game_mode, winrate_class, winrate_pct};

const MODE_ORDER: &[&str] = &[
    "Escort",
    "Hybrid",
    "Control",
    "Push",
    "Flashpoint",
    "Clash",
    "Other",
];

/// Mode identity for labels only (never bars).
fn mode_label_color(mode: &str) -> &'static str {
    match mode {
        "Escort" => "var(--chart-5)",
        "Hybrid" => "var(--chart-1)",
        "Control" => "var(--chart-2)",
        "Push" => "var(--chart-3)",
        "Flashpoint" => "var(--chart-6)",
        "Clash" => "var(--chart-4)",
        _ => "var(--text-3)",
    }
}

fn wr_bar_accent() -> &'static str {
    "var(--accent)"
}

pub(super) fn maps_tab(maps: ApiResource<Vec<MapStats>>) -> Element {
    let err = maps.error.read().clone();
    let data = maps.data.read();
    let list = data.as_ref().and_then(|d| d.as_ref());
    match list {
        None if err.is_some() => load_error_state("map stats", maps.refresh),
        None => rsx! { p { class: "loading-state", "Loading map stats..." } },
        Some(list) if list.is_empty() => rsx! {
            p { class: "empty-state", "No map stats yet." }
        },
        Some(list) => {
            let mut ranked: Vec<&MapStats> = list.iter().filter(|m| m.matches >= 3).collect();
            ranked.sort_by(|a, b| {
                let wa = winrate_pct(a.wins, a.matches);
                let wb = winrate_pct(b.wins, b.matches);
                wb.partial_cmp(&wa).unwrap_or(std::cmp::Ordering::Equal)
            });
            let best = ranked.first().copied();
            let worst = ranked.last().copied().filter(|_| ranked.len() > 1);

            let mut groups: std::collections::HashMap<&str, Vec<&MapStats>> =
                std::collections::HashMap::new();
            for m in list.iter() {
                let mode = map_game_mode(&m.map_name);
                groups.entry(mode).or_default().push(m);
            }

            let ordered_modes: Vec<&&str> = MODE_ORDER
                .iter()
                .filter(|mode| groups.contains_key(**mode))
                .collect();

            rsx! {
                if best.is_some() || worst.is_some() {
                    div { class: "map-callouts",
                        if let Some(b) = best {
                            {
                                let wr = winrate_pct(b.wins, b.matches);
                                rsx! {
                                    div { class: "map-callout best",
                                        span { class: "map-callout-label", "Best" }
                                        span { class: "map-callout-name", "{b.map_name}" }
                                        span { class: "map-callout-meta", "{wr:.0}% · {b.matches}g" }
                                    }
                                }
                            }
                        }
                        if let Some(w) = worst {
                            {
                                let wr = winrate_pct(w.wins, w.matches);
                                rsx! {
                                    div { class: "map-callout worst",
                                        span { class: "map-callout-label", "Worst" }
                                        span { class: "map-callout-name", "{w.map_name}" }
                                        span { class: "map-callout-meta", "{wr:.0}% · {w.matches}g" }
                                    }
                                }
                            }
                        }
                    }
                }

                {ordered_modes.iter().map(|&&mode| {
                    let maps_in_mode = &groups[mode];
                    let total_m: u32 = maps_in_mode.iter().map(|m| m.matches).sum();
                    let total_w: u32 = maps_in_mode.iter().map(|m| m.wins).sum();
                    let mode_wr = winrate_pct(total_w, total_m);
                    let mode_wr_cls = winrate_class(mode_wr);
                    let mc = mode_label_color(mode);

                    let mut sorted_maps: Vec<&&MapStats> = maps_in_mode.iter().collect();
                    sorted_maps.sort_by_key(|b| std::cmp::Reverse(b.matches));

                    let bar_entries: Vec<BarEntry> = sorted_maps.iter().map(|m| {
                        let wr = winrate_pct(m.wins, m.matches);
                        BarEntry {
                            label: m.map_name.clone(),
                            value: wr,
                            color: wr_bar_accent().to_string(),
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
        }
    }
}
