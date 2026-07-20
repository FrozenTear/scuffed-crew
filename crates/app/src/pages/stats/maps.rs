use dioxus::prelude::*;

use crate::components::charts::{BarEntry, HBarChart};
use crate::hooks::ApiResource;

use super::{MapStats, load_error_state, winrate_class, winrate_pct};

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

/// Return a CSS variable reference for map mode color (chart palette).
fn mode_color(mode: &str) -> &'static str {
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
            // Group by game mode
            let mut groups: std::collections::HashMap<&str, Vec<&MapStats>> =
                std::collections::HashMap::new();
            for m in list.iter() {
                let mode = map_game_mode(&m.map_name);
                groups.entry(mode).or_default().push(m);
            }

            // Sort modes by MODE_ORDER
            let ordered_modes: Vec<&&str> = MODE_ORDER
                .iter()
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
        }
    }
}
