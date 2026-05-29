use dioxus::prelude::*;

use stat_tracker::config::Config;
use stat_tracker::storage::{LocalStore, PersonalMatch};

#[component]
pub fn ProgressionPanel() -> Element {
    let config = use_signal(|| Config::load().unwrap_or_default());
    let mut refresh_tick = use_signal(|| 0u32);
    let mut selected_session: Signal<Option<String>> = use_signal(|| None);

    let data = use_resource(move || {
        let data_dir = config().data_dir.clone();
        let sid = selected_session();
        let _tick = refresh_tick();
        async move {
            let store = match LocalStore::open(&data_dir).await {
                Ok(s) => s,
                Err(_) => return (Vec::new(), Vec::new()),
            };
            let sessions = store.get_multi_capture_sessions().await.unwrap_or_default();
            let snaps = match sid {
                Some(ref session_id) => store
                    .get_session_snapshots(session_id)
                    .await
                    .unwrap_or_default(),
                None => Vec::new(),
            };
            (sessions, snaps)
        }
    });

    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            refresh_tick += 1;
        }
    });

    rsx! {
        div { class: "panel panel-wide",
            h2 { "Match Progression" }

            match &*data.read() {
                Some((s, _)) if s.is_empty() => rsx! {
                    div { class: "card",
                        p { class: "text-dim",
                            "No multi-capture sessions yet. Press Tab multiple times during a match to track stat progression."
                        }
                    }
                },
                Some((s, snapshots_data)) => rsx! {
                    div { class: "card",
                        h3 { "Sessions" }
                        div { class: "session-list",
                            for session in s.iter() {
                                {
                                    let sid = session.session_id.clone();
                                    let is_selected = selected_session().as_deref() == Some(&sid);
                                    let class_name = if is_selected { "session-item session-item-active" } else { "session-item" };
                                    let dt: chrono::DateTime<chrono::Utc> = session.started_at.clone().into();
                                    let local = dt.with_timezone(&chrono::Local);
                                    let time_str = local.format("%m/%d %H:%M").to_string();
                                    let captures = session.capture_count;
                                    let hero = session.hero.clone();
                                    let outcome = session.final_outcome.clone();
                                    rsx! {
                                        div {
                                            class: "{class_name}",
                                            onclick: move |_| selected_session.set(Some(sid.clone())),
                                            span { class: "session-hero", "{hero}" }
                                            span { class: "session-meta text-dim",
                                                "{captures} captures · {outcome} · {time_str}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if selected_session().is_some() {
                        if snapshots_data.len() >= 2 {
                            ProgressionCharts { snapshots: snapshots_data.clone() }
                        } else if snapshots_data.len() == 1 {
                            div { class: "card",
                                p { class: "text-dim", "Only one capture in this session — need at least 2 for progression." }
                            }
                        } else {
                            div { class: "card",
                                p { class: "text-dim", "No snapshots found." }
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
struct ProgressionChartsProps {
    snapshots: Vec<PersonalMatch>,
}

#[allow(non_snake_case)]
fn ProgressionCharts(props: ProgressionChartsProps) -> Element {
    let snaps = &props.snapshots;

    rsx! {
        div { class: "card",
            h3 { "Stat Progression" }
            div { class: "chart-grid",
                StatChart {
                    label: "Eliminations",
                    values: snaps.iter().map(|s| s.elims as f64).collect(),
                    color: "#7c3aed",
                }
                StatChart {
                    label: "Deaths",
                    values: snaps.iter().map(|s| s.deaths as f64).collect(),
                    color: "#ef4444",
                }
                StatChart {
                    label: "Assists",
                    values: snaps.iter().map(|s| s.assists as f64).collect(),
                    color: "#22c55e",
                }
                StatChart {
                    label: "Damage",
                    values: snaps.iter().map(|s| s.damage as f64).collect(),
                    color: "#f59e0b",
                }
                StatChart {
                    label: "Healing",
                    values: snaps.iter().map(|s| s.healing as f64).collect(),
                    color: "#06b6d4",
                }
                StatChart {
                    label: "Mitigation",
                    values: snaps.iter().map(|s| s.mitigation as f64).collect(),
                    color: "#8b5cf6",
                }
            }
        }

        div { class: "card",
            h3 { "Capture Timeline" }
            div { class: "timeline-table",
                div { class: "timeline-header",
                    span { class: "col-capture", "#" }
                    span { class: "col-stat", "E" }
                    span { class: "col-stat", "D" }
                    span { class: "col-stat", "A" }
                    span { class: "col-stat", "DMG" }
                    span { class: "col-stat", "HLG" }
                    span { class: "col-stat", "MIT" }
                    span { class: "col-time", "Time" }
                }
                for (i, snap) in snaps.iter().enumerate() {
                    {
                        let dt: chrono::DateTime<chrono::Utc> = snap.played_at.clone().into();
                        let local = dt.with_timezone(&chrono::Local);
                        let time_str = local.format("%H:%M:%S").to_string();
                        let num = i + 1;
                        rsx! {
                            div { class: "timeline-row",
                                span { class: "col-capture", "{num}" }
                                span { class: "col-stat", "{snap.elims}" }
                                span { class: "col-stat", "{snap.deaths}" }
                                span { class: "col-stat", "{snap.assists}" }
                                span { class: "col-stat", "{snap.damage}" }
                                span { class: "col-stat", "{snap.healing}" }
                                span { class: "col-stat", "{snap.mitigation}" }
                                span { class: "col-time text-dim", "{time_str}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, PartialEq, Props)]
struct StatChartProps {
    label: String,
    values: Vec<f64>,
    color: String,
}

#[allow(non_snake_case)]
fn StatChart(props: StatChartProps) -> Element {
    let values = &props.values;
    if values.is_empty() {
        return rsx! {};
    }

    let min_val = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_val = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = if (max_val - min_val).abs() < 0.001 {
        1.0
    } else {
        max_val - min_val
    };

    let width = 200.0_f64;
    let height = 60.0_f64;
    let padding = 4.0_f64;
    let usable_w = width - padding * 2.0;
    let usable_h = height - padding * 2.0;

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
    let last_val = values.last().copied().unwrap_or(0.0);
    let first_val = values.first().copied().unwrap_or(0.0);
    let delta = last_val - first_val;
    let delta_str = if delta >= 0.0 {
        format!("+{delta:.0}")
    } else {
        format!("{delta:.0}")
    };

    let color = props.color.clone();
    let svg_content = format!(
        r#"<svg viewBox="0 0 {width} {height}" xmlns="http://www.w3.org/2000/svg" class="progression-svg">
            <polyline points="{polyline_points}" fill="none" stroke="{color}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>"#
    );

    rsx! {
        div { class: "stat-chart",
            div { class: "stat-chart-header",
                span { class: "stat-chart-label", "{props.label}" }
                span { class: "stat-chart-value", "{last_val:.0}" }
                span { class: "stat-chart-delta", "{delta_str}" }
            }
            div { dangerous_inner_html: "{svg_content}" }
        }
    }
}
