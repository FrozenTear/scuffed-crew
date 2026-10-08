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
    let maps_owned: Vec<MapStats> = maps_guard
        .as_ref()
        .and_then(|d| d.as_ref())
        .cloned()
        .unwrap_or_default();
    let chips = mode_chips(&maps_owned);

    let form_guard = form.data.read();
    let form_rows: Vec<_> = form_guard
        .as_ref()
        .and_then(|d| d.as_ref())
        .map(|p| p.data.clone())
        .unwrap_or_default();

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
                    p { class: "empty-state",
                        if maps.error.read().is_some() {
                            "Couldn't load maps."
                        } else if maps.data.read().is_none() {
                            "Loading maps…"
                        } else {
                            "No map data yet."
                        }
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
                p { class: "empty-state",
                    if form.error.read().is_some() {
                        "Couldn't load recent matches."
                    } else if form.data.read().is_none() {
                        "Loading form…"
                    } else {
                        "No recent matches."
                    }
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
