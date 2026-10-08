use dioxus::prelude::*;

use scuffed_types::RoleStats;

use crate::components::charts::{DonutChart, DonutSegment};
use crate::hooks::ApiResource;

use super::role::{RolePanel, role_aggs_from_rows, role_panel};
use super::{
    HeroStats, MIN_GAMES, MapStats, MatchPage, load_error_state, map_game_mode, winrate_pct,
};

/// Role-card win-rate text: plain text tokens (no traffic light); low-sample
/// roles render muted.
fn wr_value_class(matches: u32) -> &'static str {
    if matches < MIN_GAMES {
        "role-card-wr muted"
    } else {
        "role-card-wr"
    }
}

/// Tracker outcomes are `victory` / `defeat` / `draw` (`win`/`loss` accepted
/// as aliases) — same contract as history's `outcome_class`.
fn outcome_chip_class(outcome: &str) -> &'static str {
    match outcome.to_lowercase().as_str() {
        "victory" | "win" => "form-chip win",
        "defeat" | "loss" => "form-chip loss",
        _ => "form-chip draw",
    }
}

/// Mode WR aggregate for overview chips.
struct ModeChip {
    name: &'static str,
    matches: u32,
    wr: f64,
}

fn mode_chips(maps: &[MapStats]) -> Vec<ModeChip> {
    const ORDER: &[&str] = &["Escort", "Hybrid", "Control", "Push", "Flashpoint", "Clash"];
    let mut out = Vec::new();
    for &mode in ORDER {
        let mut m = 0u32;
        let mut w = 0u32;
        for map in maps {
            if map_game_mode(&map.map_name) == mode {
                m += map.matches;
                w += map.wins;
            }
        }
        if m > 0 {
            out.push(ModeChip {
                name: mode,
                matches: m,
                wr: winrate_pct(w, m),
            });
        }
    }
    out
}

pub(super) fn overview_tab(
    heroes: ApiResource<Vec<HeroStats>>,
    roles: ApiResource<Vec<RoleStats>>,
    maps: ApiResource<Vec<MapStats>>,
    form: ApiResource<MatchPage>,
) -> Element {
    let role_err = roles.error.read().clone();
    let role_data = roles.data.read();
    let rows = role_data.as_ref().and_then(|d| d.as_ref());
    let phase = role_panel(role_err.as_deref(), rows.map(|rows| rows.as_slice()));
    let aggs = match phase {
        RolePanel::Ready(rows) => role_aggs_from_rows(rows),
        RolePanel::Pending | RolePanel::Failed => Vec::new(),
    };
    let total_matches: u32 = aggs.iter().map(|r| r.matches).sum();
    let donut_segments: Vec<DonutSegment> = aggs
        .iter()
        .filter(|r| r.matches > 0)
        .map(|r| DonutSegment {
            label: r.name.to_string(),
            value: r.matches as f64,
            color: r.color.to_string(),
        })
        .collect();

    let h_err = heroes.error.read().clone();
    let h_data = heroes.data.read();
    let h_list = h_data.as_ref().and_then(|d| d.as_ref());
    let top5: Vec<_> = h_list
        .map(|list| {
            let mut by_wr: Vec<_> = list.iter().filter(|h| h.matches >= MIN_GAMES).collect();
            by_wr.sort_by(|a, b| {
                let wa = winrate_pct(a.wins, a.matches);
                let wb = winrate_pct(b.wins, b.matches);
                wb.partial_cmp(&wa).unwrap_or(std::cmp::Ordering::Equal)
            });
            by_wr.into_iter().take(5).collect()
        })
        .unwrap_or_default();

    let maps_guard = maps.data.read();
    let maps_loaded = maps_guard.as_ref().and_then(|d| d.as_ref());
    // A held fetch finishes as `Some(None)`. Flattening treats that as still
    // loading, the same way the heroes tab does. An empty list is the only
    // "no data" state.
    let maps_waiting = maps_loaded.is_none();
    let maps_owned: Vec<MapStats> = maps_loaded.cloned().unwrap_or_default();
    let chips = mode_chips(&maps_owned);
    drop(maps_guard);

    let form_guard = form.data.read();
    let form_loaded = form_guard.as_ref().and_then(|d| d.as_ref());
    let form_waiting = form_loaded.is_none();
    let form_rows: Vec<_> = form_loaded.map(|p| p.data.clone()).unwrap_or_default();
    drop(form_guard);

    rsx! {
        div { class: "overview-grid",
            if matches!(phase, RolePanel::Ready(_)) && total_matches > 0 {
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
                        {
                            aggs.iter()
                                .filter(|r| r.matches > 0)
                                .map(|r| {
                                    let wr = r.winrate();
                                    let wr_cls = wr_value_class(r.matches);
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
                                })
                        }
                    }
                }
            } else {
                div { class: "overview-section",
                    h3 { "Roles" }
                    match phase {
                        RolePanel::Pending => rsx! {
                            p { class: "loading-state", "Loading role breakdown..." }
                        },
                        RolePanel::Failed => load_error_state("role breakdown", roles.refresh),
                        RolePanel::Ready(_) => rsx! {
                            p { class: "empty-state", "No role stats yet." }
                        },
                    }
                }
            }
            div { class: "overview-section",
                h3 { "Top Heroes (3+ matches)" }
                if h_err.is_some() {
                    {load_error_state("hero rankings", heroes.refresh)}
                } else if h_list.is_none() {
                    p { class: "loading-state", "Loading hero rankings..." }
                } else if top5.is_empty() {
                    p { class: "empty-state", "Need 3+ games on a hero for rankings." }
                } else {
                    div { class: "mini-hero-list",
                        {
                            top5.iter()
                                .map(|h| {
                                    let wr = winrate_pct(h.wins, h.matches);
                                    let name = h.hero.clone();
                                    let m = h.matches;
                                    rsx! {
                                        div { class: "mini-hero-row", key: "{name}",
                                            span { class: "mini-hero-name", "{name}" }
                                            span { class: "mini-hero-meta", "{m}g" }
                                            span { class: "mini-hero-wr", "{wr:.0}%" }
                                        }
                                    }
                                })
                        }
                    }
                }
            }
            div { class: "overview-section",
                h3 { "Mode Win Rates" }
                if chips.is_empty() {
                    if maps.error.read().is_some() {
                        p { class: "empty-state", "Couldn't load maps." }
                    } else if maps_waiting {
                        p { class: "loading-state", "Loading maps…" }
                    } else {
                        p { class: "empty-state", "No map data yet." }
                    }
                } else {
                    div { class: "mode-chips",
                        {
                            chips
                                .iter()
                                .map(|c| {
                                    let name = c.name;
                                    let wr = c.wr;
                                    let m = c.matches;
                                    rsx! {
                                        div { class: "mode-chip", key: "{name}",
                                            span { class: "mode-chip-name", "{name}" }
                                            span { class: "mode-chip-wr", "{wr:.0}%" }
                                            span { class: "mode-chip-n", "{m}g" }
                                        }
                                    }
                                })
                        }
                    }
                }
            }
        }

        div { class: "overview-section overview-form",
            h3 { "Recent form" }
            if form_rows.is_empty() {
                if form.error.read().is_some() {
                    p { class: "empty-state", "Couldn't load recent matches." }
                } else if form_waiting {
                    p { class: "loading-state", "Loading form…" }
                } else {
                    p { class: "empty-state", "No recent matches." }
                }
            } else {
                div { class: "form-strip",
                    {
                        form_rows
                            .iter()
                            .take(10)
                            .map(|m| {
                                let oc = outcome_chip_class(&m.outcome);
                                let label = match m.outcome.to_lowercase().as_str() {
                                    "victory" | "win" => "W",
                                    "defeat" | "loss" => "L",
                                    _ => "D",
                                };
                                let id = m.id.clone();
                                let tip = format!("{} · {}", m.hero, m.map_name);
                                rsx! {
                                    span { class: "{oc}", key: "{id}", title: "{tip}", "{label}" }
                                }
                            })
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::ApiResource;

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

    fn pump(dom: &mut VirtualDom) {
        for _ in 0..8 {
            dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        }
    }

    #[test]
    fn held_overview_fetches_stay_on_loading_text() {
        fn view() -> Element {
            let heroes = resource_of(None::<Vec<HeroStats>>);
            let roles = resource_of(None::<Vec<RoleStats>>);
            let maps = resource_of(None::<Vec<MapStats>>);
            let form = resource_of(None::<MatchPage>);
            overview_tab(heroes, roles, maps, form)
        }

        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        pump(&mut dom);
        let html = dioxus_ssr::render(&dom);
        assert!(
            html.contains("Loading maps"),
            "a finished held maps fetch must stay on loading: {html}"
        );
        assert!(
            html.contains("Loading form"),
            "a finished held form fetch must stay on loading: {html}"
        );
        assert!(
            !html.contains("No map data yet."),
            "held maps must not look empty: {html}"
        );
        assert!(
            !html.contains("No recent matches."),
            "held form must not look empty: {html}"
        );
        assert!(
            html.contains("loading-state"),
            "a held fetch uses the loading class: {html}"
        );
        assert!(
            !html.contains("empty-state"),
            "a held fetch must not use the empty class: {html}"
        );
    }

    #[test]
    fn empty_overview_lists_say_there_is_no_data() {
        fn view() -> Element {
            let heroes = resource_of(Some(Vec::<HeroStats>::new()));
            let roles = resource_of(Some(Vec::<RoleStats>::new()));
            let maps = resource_of(Some(Vec::<MapStats>::new()));
            let form = resource_of(Some(MatchPage {
                data: Vec::new(),
                next_cursor: None,
            }));
            overview_tab(heroes, roles, maps, form)
        }

        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        pump(&mut dom);
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains("No map data yet."), "{html}");
        assert!(html.contains("No recent matches."), "{html}");
        assert!(html.contains("empty-state"), "{html}");
        assert!(!html.contains("Loading maps"), "{html}");
        assert!(!html.contains("Loading form"), "{html}");
    }
}
