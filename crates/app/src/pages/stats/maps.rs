use dioxus::prelude::*;

use crate::components::charts::{BarEntry, HBarChart};
use crate::components::ui::{EmptyCard, SkeletonBars, btn_class};
use crate::hooks::ApiResource;
use crate::pages::stats_member::{
    STATS_VIEWS_CSS, STATS_VIEWS_CSS_ID, stats_load_error, status_or_refusal,
};

use super::{MapStats, map_game_mode, wr_bar_color, wr_text_class};
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

/// Mode identity colors appear ONLY on the swatch next to the section title.
/// The title itself is plain text ink, so the name never rests on the colour.
fn mode_label_color(mode: &str) -> &'static str {
    match mode {
        "Escort" => "var(--chart-5)",
        "Hybrid" => "var(--chart-1)",
        "Control" => "var(--chart-2)",
        "Push" => "var(--chart-3)",
        "Flashpoint" => "var(--chart-6)",
        "Clash" => "var(--chart-4)",
        _ => "var(--text-subtle)",
    }
}

/// Styles of the Maps tab only. The shared pieces are in `STATS_VIEWS_CSS`.
const MAPS_CSS: &str = r#"
.map-mode-title {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
}
.map-mode-swatch {
  flex: 0 0 auto;
  width: var(--space-3);
  height: var(--space-3);
  border-radius: var(--radius-pill);
}
.map-mode-agg {
  margin: 0;
  color: var(--text-3);
  font-size: var(--text-sm);
  font-variant-numeric: tabular-nums;
}
.map-pick-kind {
  font-family: var(--font-body);
  font-size: var(--text-xs);
  font-weight: var(--font-label-weight);
  letter-spacing: var(--font-head-tracking);
  text-transform: uppercase;
  color: var(--text-subtle);
}
@media (max-width: 720px) {
  .map-picks.stats-view-tiles { grid-auto-flow: row; }
}
"#;

/// Id-safe form of a mode name, for the heading id of its section.
fn mode_id(mode: &str) -> String {
    mode.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// What screen readers hear for the tab. Always rendered, only the text
/// changes.
fn maps_status(loaded: Option<usize>, failed: bool) -> String {
    match loaded {
        // The failure card is an alert that says it; the status line stays
        // quiet so it is not announced twice.
        None if failed => String::new(),
        None => "Loading map stats.".to_string(),
        Some(0) => "No map stats yet.".to_string(),
        Some(1) => "Showing 1 map.".to_string(),
        Some(n) => format!("Showing {n} maps."),
    }
}

/// "No map stats yet", with the next step: connect the tracker.
#[component]
fn MapsEmpty() -> Element {
    rsx! {
        EmptyCard {
            title: "No map stats yet",
            message: "Map results show up here after the tracker uploads your first games.",
            actions: rsx! {
                // The install steps are on this page, as the summary card's
                // link says.
                a { class: btn_class(true), href: "{super::TRACKER_ANCHOR}", "Set up the tracker" }
            },
        }
    }
}

/// One Best or Worst tile: the rate over the label, as on the stats frame.
/// "Best" and "Worst" are words, so neither rests on a colour.
fn map_callout(kind: &'static str, m: &MapStats, wr: f64) -> Element {
    rsx! {
        div { class: "stats-view-tile map-pick",
            dt { class: "map-pick-kind", "{kind} map" }
            dd { class: "stats-view-tile-value", "{m.map_name}" }
            dd { class: "stats-view-tile-label", "{wr:.0}% win rate, {m.matches} games" }
        }
    }
}

pub(super) fn maps_tab(maps: ApiResource<Vec<MapStats>>) -> Element {
    let failed = maps.error.read().is_some();
    let data = maps.data.read();
    let list = data.as_ref().and_then(|d| d.as_ref());
    let map_err = maps.error.read().clone();
    let status = status_or_refusal(
        maps_status(list.map(Vec::len), failed),
        "map stats",
        map_err.as_deref().filter(|_| list.is_none()),
    );
    let body = match list {
        None if failed => stats_load_error("map stats", maps.error.read().clone(), maps.refresh),
        None => rsx! { SkeletonBars { rows: 4 } },
        Some(list) if list.is_empty() => rsx! { MapsEmpty {} },
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
                p { class: "stats-view-note", {MAP_RULE_NOTE} }

                if best.is_some() || worst.is_some() {
                    dl { class: "stats-view-tiles map-picks",
                        if let Some((b, wr)) = best {
                            {map_callout("Best", b, wr)}
                        }
                        if let Some((w, wr)) = worst {
                            {map_callout("Worst", w, wr)}
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
                    let title_id = format!("maps-mode-{}-title", mode_id(mode));

                    let mut sorted_maps: Vec<&&MapStats> = maps_in_mode.iter().collect();
                    sorted_maps.sort_by_key(|b| std::cmp::Reverse(b.matches));

                    // Each bar repeats the number written next to it.
                    let bar_entries: Vec<BarEntry> = sorted_maps.iter().map(|&&m| map_bar(m)).collect();

                    rsx! {
                        section { class: "ui-stack map-mode", key: "{mode}", aria_labelledby: "{title_id}",
                            div { class: "stats-view-section-head",
                                h2 { id: "{title_id}", class: "ui-section-title map-mode-title",
                                    span {
                                        class: "map-mode-swatch",
                                        style: "background:{mc};",
                                        aria_hidden: "true",
                                    }
                                    "{mode}"
                                }
                                p { class: "map-mode-agg",
                                    {format!("{} · ", crate::util::pluralize(total_m as usize, "match", "matches"))}
                                    span { class: "{mode_wr_cls}", "{mode_result}" }
                                }
                            }
                            div { class: "stats-view-card",
                                HBarChart {
                                    entries: bar_entries,
                                    max: Some(100.0),
                                    reference: Some(50.0),
                                }
                            }
                        }
                    }
                })}
            }
        }
    };

    rsx! {
        crate::styles::StyleOnce { id: STATS_VIEWS_CSS_ID, css: STATS_VIEWS_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-stats-maps-css", css: MAPS_CSS }
        div { class: "stats-view-body",
            p { class: "visually-hidden", role: "status", "{status}" }
            {body}
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

    fn resource_of<T: Clone + 'static>(value: Option<T>) -> ApiResource<T> {
        let refresh = use_signal(|| 0u64);
        let error = use_signal(|| None::<String>);
        let truncated = use_signal(|| false);
        let shown = use_signal(|| 0usize);
        let page_budget = use_signal(|| 1usize);
        let data = use_resource(move || {
            let value = value.clone();
            async move { value }
        });
        ApiResource {
            data,
            refresh,
            error,
            truncated,
            shown,
            page_budget,
        }
    }

    fn render(app: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        for _ in 0..8 {
            dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        }
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn status_line_names_every_state() {
        assert_eq!(maps_status(None, false), "Loading map stats.");
        assert_eq!(maps_status(None, true), "", "the alert says it once");
        assert_eq!(maps_status(Some(0), false), "No map stats yet.");
        assert_eq!(maps_status(Some(1), false), "Showing 1 map.");
        assert_eq!(maps_status(Some(7), false), "Showing 7 maps.");
    }

    #[test]
    fn one_match_is_singular_and_empty_links_to_the_steps_on_this_page() {
        fn one() -> Element {
            maps_tab(resource_of(Some(vec![map(1, 1)])))
        }
        let html = render(one);
        assert!(html.contains("1 match · "), "{html}");
        assert!(!html.contains("1 matches"), "{html}");
        fn none() -> Element {
            maps_tab(resource_of(Some(Vec::<MapStats>::new())))
        }
        let html = render(none);
        assert!(html.contains("href=\"#stats-tracker\""), "{html}");
        assert!(!html.contains("href=\"/stats/tokens\""), "{html}");
    }

    #[test]
    fn mode_ids_come_from_the_mode_name() {
        assert_eq!(mode_id("Escort"), "escort");
        assert_eq!(mode_id("Payload Race"), "payload-race");
    }

    #[test]
    fn best_and_worst_are_words_and_mode_titles_are_plain_ink() {
        fn view() -> Element {
            let mut busan = map(20, 4);
            busan.map_name = "Busan".to_string();
            let maps = resource_of(Some(vec![map(10, 8), busan]));
            maps_tab(maps)
        }
        let html = render(view);
        assert!(!html.contains("<h1") && !html.contains("<main"), "{html}");
        assert!(
            html.contains("role=\"status\">Showing 2 maps.</p>"),
            "{html}"
        );
        assert!(html.contains(">Best map</dt>"), "{html}");
        assert!(html.contains(">Worst map</dt>"), "{html}");
        assert!(html.contains("80% win rate, 10 games"), "{html}");
        assert!(html.contains("20% win rate, 20 games"), "{html}");
        // The mode colour sits on a hidden swatch, never on the title text.
        assert!(
            html.contains("id=\"maps-mode-control-title\" class=\"ui-section-title map-mode-title\"><span class=\"map-mode-swatch\" style=\"background:var(--chart-2);\" aria-hidden=\"true\"></span>Control</h2>"),
            "{html}"
        );
        assert!(
            html.contains("aria-labelledby=\"maps-mode-control-title\""),
            "{html}"
        );
        // Every bar carries its number as text.
        assert!(html.contains("80.0% (10 games)"), "{html}");
        assert!(html.contains("20.0% (20 games)"), "{html}");
        assert!(
            !html.contains('\u{2014}') && !html.contains('\u{2013}'),
            "{html}"
        );
    }
}
