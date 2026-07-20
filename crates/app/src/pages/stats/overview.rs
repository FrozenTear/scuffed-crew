use dioxus::prelude::*;

use crate::components::charts::{DonutChart, DonutSegment};
use crate::hooks::ApiResource;

use super::{HeroStats, MapStats, MatchPage, load_error_state, map_game_mode, winrate_pct};

// -- Role aggregation --

pub(super) struct RoleAgg {
    pub name: &'static str,
    pub color: &'static str,
    pub matches: u32,
    pub wins: u32,
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

    pub fn winrate(&self) -> f64 {
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

pub(super) fn hero_to_role(name: &str) -> &'static str {
    match name {
        "D.Va" | "Domina" | "Doomfist" | "Hazard" | "Junker Queen" | "Mauga" | "Orisa"
        | "Ramattra" | "Reinhardt" | "Roadhog" | "Sigma" | "Winston" | "Wrecking Ball"
        | "Zarya" => "Tank",
        "Ana" | "Baptiste" | "Brigitte" | "Illari" | "Juno" | "Kiriko" | "Lifeweaver" | "Lúcio"
        | "Mercy" | "Mizuki" | "Moira" | "Wuyang" | "Zenyatta" => "Support",
        _ => "Damage",
    }
}

pub(super) fn aggregate_roles(heroes: &[HeroStats]) -> Vec<RoleAgg> {
    let mut tank = RoleAgg::new("Tank", "var(--chart-5)");
    let mut damage = RoleAgg::new("Damage", "var(--chart-4)");
    let mut support = RoleAgg::new("Support", "var(--chart-2)");

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

fn wr_value_class(pct: f64) -> &'static str {
    if pct >= 55.0 {
        "role-card-wr high"
    } else if pct >= 45.0 {
        "role-card-wr mid"
    } else {
        "role-card-wr low"
    }
}

fn outcome_chip_class(outcome: &str) -> &'static str {
    match outcome.to_lowercase().as_str() {
        "win" => "form-chip win",
        "loss" => "form-chip loss",
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
    maps: ApiResource<Vec<MapStats>>,
    form: ApiResource<MatchPage>,
) -> Element {
    let h_err = heroes.error.read().clone();
    let h_data = heroes.data.read();
    let h_list = h_data.as_ref().and_then(|d| d.as_ref());

    match h_list {
        None if h_err.is_some() => load_error_state("role breakdown", heroes.refresh),
        None => rsx! { p { class: "loading-state", "Loading role breakdown..." } },
        Some(list) if list.is_empty() => rsx! {
            p { class: "empty-state", "Play some matches to see your role breakdown." }
        },
        Some(list) => {
            let roles = aggregate_roles(list);
            let total_matches: u32 = roles.iter().map(|r| r.matches).sum();
            let donut_segments: Vec<DonutSegment> = roles
                .iter()
                .filter(|r| r.matches > 0)
                .map(|r| DonutSegment {
                    label: r.name.to_string(),
                    value: r.matches as f64,
                    color: r.color.to_string(),
                })
                .collect();

            // Top heroes mini (min 3 games, by WR, top 5)
            let mut by_wr: Vec<_> = list.iter().filter(|h| h.matches >= 3).collect();
            by_wr.sort_by(|a, b| {
                let wa = winrate_pct(a.wins, a.matches);
                let wb = winrate_pct(b.wins, b.matches);
                wb.partial_cmp(&wa).unwrap_or(std::cmp::Ordering::Equal)
            });
            let top5: Vec<_> = by_wr.into_iter().take(5).collect();

            // Mode chips from maps resource (best-effort)
            let maps_guard = maps.data.read();
            let maps_owned: Vec<MapStats> = maps_guard
                .as_ref()
                .and_then(|d| d.as_ref())
                .cloned()
                .unwrap_or_default();
            let chips = mode_chips(&maps_owned);

            // Form strip
            let form_guard = form.data.read();
            let form_rows: Vec<_> = form_guard
                .as_ref()
                .and_then(|d| d.as_ref())
                .map(|p| p.data.clone())
                .unwrap_or_default();

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
                    div { class: "overview-section",
                        h3 { "Top Heroes (3+ matches)" }
                        if top5.is_empty() {
                            p { class: "empty-state", "Need 3+ games on a hero for rankings." }
                        } else {
                            div { class: "mini-hero-list",
                                {top5.iter().map(|h| {
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
                                })}
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
                                {chips.iter().map(|c| {
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
                                })}
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
                            {form_rows.iter().take(10).map(|m| {
                                let oc = outcome_chip_class(&m.outcome);
                                let label = match m.outcome.to_lowercase().as_str() {
                                    "win" => "W",
                                    "loss" => "L",
                                    _ => "D",
                                };
                                let id = m.id.clone();
                                let tip = format!("{} · {}", m.hero, m.map_name);
                                rsx! {
                                    span { class: "{oc}", key: "{id}", title: "{tip}", "{label}" }
                                }
                            })}
                        }
                    }
                }
            }
        }
    }
}
