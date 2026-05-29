use dioxus::prelude::*;

const CHARTS_CSS: &str = r#"
    .donut-wrap {
        display: flex;
        align-items: center;
        gap: 1.5rem;
        flex-wrap: wrap;
    }
    .donut-outer {
        position: relative;
        flex-shrink: 0;
    }
    .donut-ring {
        width: 140px;
        height: 140px;
        border-radius: 50%;
    }
    .donut-center {
        position: absolute;
        top: 50%;
        left: 50%;
        transform: translate(-50%, -50%);
        text-align: center;
        pointer-events: none;
    }
    .donut-center-value {
        font-family: var(--font-display);
        font-size: 1.2rem;
        font-weight: 700;
        color: var(--text-bright);
        line-height: 1.1;
    }
    .donut-center-label {
        font-size: 0.6rem;
        color: var(--text-muted);
        text-transform: uppercase;
        letter-spacing: 0.06em;
    }
    .donut-legend {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
    }
    .legend-row {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        font-size: 0.85rem;
    }
    .legend-dot {
        width: 10px;
        height: 10px;
        border-radius: 2px;
        flex-shrink: 0;
    }
    .legend-name { color: var(--text-secondary); }
    .legend-val {
        margin-left: auto;
        color: var(--text-muted);
        font-size: 0.8rem;
        padding-left: 0.5rem;
    }

    .hbar-list { display: flex; flex-direction: column; gap: 0.4rem; }
    .hbar-row { display: flex; align-items: center; gap: 0.75rem; }
    .hbar-label {
        min-width: 110px;
        font-size: 0.8rem;
        color: var(--text-secondary);
        text-align: right;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .hbar-track {
        flex: 1;
        height: 20px;
        background: var(--bg-card);
        border-radius: 4px;
        overflow: hidden;
    }
    .hbar-fill {
        height: 100%;
        border-radius: 4px;
        transition: width 0.3s ease;
        min-width: 2px;
    }
    .hbar-val {
        min-width: 50px;
        font-size: 0.8rem;
        font-weight: 600;
    }
    .hbar-val.high { color: #34d399; }
    .hbar-val.mid { color: #fbbf24; }
    .hbar-val.low { color: #f87171; }
"#;

#[derive(Clone, PartialEq)]
pub struct DonutSegment {
    pub label: String,
    pub value: f64,
    pub color: String,
}

#[component]
pub fn DonutChart(
    segments: Vec<DonutSegment>,
    #[props(default)] center_value: Option<String>,
    #[props(default)] center_label: Option<String>,
) -> Element {
    let total: f64 = segments.iter().map(|s| s.value).sum();
    if total == 0.0 {
        return rsx! { p { class: "empty-state", "No data" } };
    }

    let active: Vec<&DonutSegment> = segments.iter().filter(|s| s.value > 0.0).collect();

    let gradient = {
        let mut stops = Vec::new();
        let mut cum = 0.0_f64;
        for seg in &active {
            let deg = (seg.value / total) * 360.0;
            let end = cum + deg;
            stops.push(format!("{} {cum:.2}deg {end:.2}deg", seg.color));
            cum = end;
        }
        stops.join(", ")
    };

    let ring_style = format!(
        "width:140px;height:140px;border-radius:50%;\
         background:conic-gradient({gradient});\
         -webkit-mask:radial-gradient(circle,transparent 42%,black 42%);\
         mask:radial-gradient(circle,transparent 42%,black 42%);"
    );

    rsx! {
        style { {CHARTS_CSS} }
        div { class: "donut-wrap",
            div { class: "donut-outer",
                div { class: "donut-ring", style: "{ring_style}" }
                if center_value.is_some() || center_label.is_some() {
                    div { class: "donut-center",
                        if let Some(ref v) = center_value {
                            div { class: "donut-center-value", "{v}" }
                        }
                        if let Some(ref l) = center_label {
                            div { class: "donut-center-label", "{l}" }
                        }
                    }
                }
            }
            div { class: "donut-legend",
                {active.iter().map(|seg| {
                    let pct = (seg.value / total) * 100.0;
                    let count = seg.value as u32;
                    let color = seg.color.clone();
                    let label = seg.label.clone();
                    rsx! {
                        div { class: "legend-row", key: "{label}",
                            div { class: "legend-dot", style: "background:{color};" }
                            span { class: "legend-name", "{label}" }
                            span { class: "legend-val", "{pct:.1}% ({count})" }
                        }
                    }
                })}
            }
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct BarEntry {
    pub label: String,
    pub value: f64,
    pub color: String,
    pub display: String,
}

#[component]
pub fn HBarChart(entries: Vec<BarEntry>) -> Element {
    let max = entries.iter().map(|e| e.value).fold(0.0_f64, f64::max);
    if max == 0.0 {
        return rsx! {};
    }

    rsx! {
        style { {CHARTS_CSS} }
        div { class: "hbar-list",
            {entries.iter().map(|entry| {
                let w = (entry.value / max) * 100.0;
                let cls = if entry.value >= 55.0 { "hbar-val high" }
                    else if entry.value >= 45.0 { "hbar-val mid" }
                    else { "hbar-val low" };
                let color = entry.color.clone();
                let label = entry.label.clone();
                let display = entry.display.clone();
                rsx! {
                    div { class: "hbar-row", key: "{label}",
                        span { class: "hbar-label", "{label}" }
                        div { class: "hbar-track",
                            div {
                                class: "hbar-fill",
                                style: "width:{w:.1}%;background:{color};",
                            }
                        }
                        span { class: "{cls}", "{display}" }
                    }
                }
            })}
        }
    }
}
