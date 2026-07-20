use dioxus::prelude::*;

use crate::components::charts::{DonutChart, DonutSegment};
use crate::hooks::ApiResource;

use super::{HeroStats, load_error_state, winrate_pct};

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
    // Role colors use chart palette tokens (var(--chart-N)) expressed as CSS
    // variable references so the DonutSegment color field renders them inline.
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

pub(super) fn overview_tab(heroes: ApiResource<Vec<HeroStats>>) -> Element {
    let err = heroes.error.read().clone();
    let data = heroes.data.read();
    let list = data.as_ref().and_then(|d| d.as_ref());
    match list {
        None if err.is_some() => load_error_state("role breakdown", heroes.refresh),
        None => rsx! { p { class: "loading-state", "Loading role breakdown..." } },
        Some(list) if list.is_empty() => rsx! {
            p { class: "empty-state", "Play some matches to see your role breakdown." }
        },
        Some(list) => {
            let roles = aggregate_roles(list);
            let total_matches: u32 = roles.iter().map(|r| r.matches).sum();
            let donut_segments: Vec<DonutSegment> = roles
                .iter()
                .map(|r| DonutSegment {
                    label: r.name.to_string(),
                    value: r.matches as f64,
                    color: r.color.to_string(),
                })
                .collect();

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
        }
    }
}
