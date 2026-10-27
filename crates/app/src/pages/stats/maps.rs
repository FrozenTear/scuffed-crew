use dioxus::prelude::*;

use crate::components::charts::{BarEntry, HBarChart};
use crate::hooks::ApiResource;

use super::{MapStats, load_error_state, map_game_mode, wr_bar_color, wr_text_class};
use crate::sample::{MAP_RULE_NOTE, TOO_FEW_GAMES, Tally, best_and_worst};

/// One map bar: the win rate from 10 decided games; below that an empty bar
/// with "Too few games" and the record, or "No games" for an empty map.
fn map_bar(m: &MapStats) -> BarEntry {
    let tally = m.tally();
    let rate = tally.win_rate_pct();
    let display = match rate {
        Some(wr) => format!("{wr:.1}% ({} games)", m.matches),
        None if m.matches == 0 => tally.label(),
        None => format!("{TOO_FEW_GAMES}, {} ({} games)", tally.label(), m.matches),
    };
    BarEntry {
        label: m.map_name.clone(),
        value: rate.unwrap_or(0.0),
        color: wr_bar_color(tally).to_string(),
        display,
        muted: rate.is_none(),
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

/// Mode identity colors appear ONLY on the section header label.
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
            // Best and worst need a win rate, so only maps with 10+ games.
            let mut ranked: Vec<(&MapStats, f64)> = list
                .iter()
                .filter_map(|m| m.tally().win_rate_pct().map(|wr| (m, wr)))
                .collect();
            ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            // Worst hides when it shows the same % as Best.
            let (best, worst) = best_and_worst(&ranked);

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
                p { class: "stats-gate-note", {MAP_RULE_NOTE} }

                if best.is_some() || worst.is_some() {
                    div { class: "map-callouts",
                        if let Some((b, wr)) = best {
                            {
                                rsx! {
                                    div { class: "map-callout best",
                                        span { class: "map-callout-label", "Best" }
                                        span { class: "map-callout-name", "{b.map_name}" }
                                        span { class: "map-callout-meta", "{wr:.0}% · {b.matches}g" }
                                    }
                                }
                            }
                        }
                        if let Some((w, wr)) = worst {
                            {
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
                    let mode_total = Tally::sum(maps_in_mode.iter().map(|m| m.tally()));
                    let total_m = mode_total.games;
                    let mode_result = mode_total.label();
                    let mode_wr_cls = wr_text_class(mode_total);
                    let mc = mode_label_color(mode);

                    let mut sorted_maps: Vec<&&MapStats> = maps_in_mode.iter().collect();
                    sorted_maps.sort_by_key(|b| std::cmp::Reverse(b.matches));

                    let bar_entries: Vec<BarEntry> = sorted_maps.iter().map(|&&m| map_bar(m)).collect();

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
                                    span { class: "{mode_wr_cls}", "{mode_result}" }
                                }
                            }
                            HBarChart {
                                entries: bar_entries,
                                max: Some(100.0),
                                reference: Some(50.0),
                            }
                        }
                    }
                })}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(matches: u32, wins: u32) -> MapStats {
        MapStats {
            map_name: "Ilios".to_string(),
            matches,
            wins,
            losses: matches - wins,
            draws: 0,
        }
    }

    #[test]
    fn map_bars_follow_the_small_sample_rule() {
        let empty = map_bar(&map(0, 0));
        assert_eq!(empty.display, "No games");
        assert_eq!(empty.value, 0.0);
        assert!(empty.muted);

        let few = map_bar(&map(9, 6));
        assert_eq!(few.display, "Too few games, 6W-3L (9 games)");
        assert_eq!(few.value, 0.0);
        assert!(few.muted);

        let mut open = map(12, 6);
        open.losses = 3;
        let open = map_bar(&open);
        assert_eq!(open.display, "Too few games, 6W-3L (12 games)");
        assert!(open.muted);

        let enough = map_bar(&map(10, 6));
        assert_eq!(enough.display, "60.0% (10 games)");
        assert_eq!(enough.value, 60.0);
        assert!(!enough.muted);
    }
}
