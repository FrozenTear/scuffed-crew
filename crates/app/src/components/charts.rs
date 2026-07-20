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
        font-family: var(--font-head);
        font-size: 1.2rem;
        font-weight: 700;
        color: var(--text);
        line-height: 1.1;
    }
    .donut-center-label {
        font-size: 0.6rem;
        color: var(--text-3);
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
    .legend-name { color: var(--text-2); }
    .legend-val {
        margin-left: auto;
        color: var(--text-3);
        font-size: 0.8rem;
        padding-left: 0.5rem;
    }

    .hbar-list { display: flex; flex-direction: column; gap: 0.4rem; }
    .hbar-row { display: flex; align-items: center; gap: 0.75rem; }
    .hbar-row.muted .hbar-fill { opacity: 0.45; }
    .hbar-label {
        min-width: 110px;
        font-size: 0.8rem;
        color: var(--text-2);
        text-align: right;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .hbar-track {
        position: relative;
        flex: 1;
        height: 20px;
        background: var(--surface);
        border-radius: 4px;
        overflow: hidden;
    }
    .hbar-fill {
        height: 100%;
        border-radius: 4px;
        transition: width 0.3s ease;
        min-width: 2px;
    }
    /* Reference hairline (e.g. the 50% winrate mark) — recessive, one shade
       off the track surface, drawn across the full track height. */
    .hbar-ref {
        position: absolute;
        top: 0;
        bottom: 0;
        width: 1px;
        background: var(--border);
        pointer-events: none;
    }
    .hbar-val {
        min-width: 50px;
        font-size: 0.8rem;
        font-weight: 600;
        color: var(--text-2);
        font-variant-numeric: tabular-nums;
    }
    .hbar-row.muted .hbar-val { color: var(--text-3); font-weight: 400; }
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
    /// Low-sample rows render recessive (reduced fill opacity, muted value text).
    pub muted: bool,
}

#[component]
pub fn HBarChart(
    entries: Vec<BarEntry>,
    /// Fixed scale maximum (e.g. `Some(100.0)` for percentages). Defaults to
    /// the data maximum, which makes bars relative to the largest entry.
    #[props(default)]
    max: Option<f64>,
    /// Draw a recessive reference hairline at this value on the track
    /// (e.g. `Some(50.0)` with `max: Some(100.0)` marks the 50% winrate line).
    #[props(default)]
    reference: Option<f64>,
) -> Element {
    let data_max = entries.iter().map(|e| e.value).fold(0.0_f64, f64::max);
    let scale_max = max.unwrap_or(data_max);
    if scale_max <= 0.0 {
        return rsx! {};
    }
    let ref_pos = reference.map(|r| (r / scale_max * 100.0).clamp(0.0, 100.0));

    rsx! {
        style { {CHARTS_CSS} }
        div { class: "hbar-list",
            {entries.iter().map(|entry| {
                let w = (entry.value / scale_max * 100.0).clamp(0.0, 100.0);
                let row_cls = if entry.muted { "hbar-row muted" } else { "hbar-row" };
                let color = entry.color.clone();
                let label = entry.label.clone();
                let display = entry.display.clone();
                rsx! {
                    div { class: "{row_cls}", key: "{label}",
                        span { class: "hbar-label", "{label}" }
                        div { class: "hbar-track",
                            div {
                                class: "hbar-fill",
                                style: "width:{w:.1}%;background:{color};",
                            }
                            if let Some(rp) = ref_pos {
                                div { class: "hbar-ref", style: "left:{rp:.1}%;" }
                            }
                        }
                        span { class: "hbar-val", "{display}" }
                    }
                }
            })}
        }
    }
}
