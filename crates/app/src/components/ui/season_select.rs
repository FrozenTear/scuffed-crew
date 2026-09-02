//! Season picker shared by every stats surface: "All time" plus each season
//! from `GET /api/public/seasons` (the current season is marked). Renders
//! nothing while no seasons exist — there is nothing to switch between.

use dioxus::prelude::*;
use scuffed_types::Season;

use crate::components::ui::Label;
use crate::hooks::use_api;

pub const ALL_TIME_LABEL: &str = "All time";

pub const SEASON_SELECT_CSS: &str = r#"
.season-select { display: flex; flex-direction: column; gap: var(--space-1); }
"#;

#[component]
pub fn SeasonSelect(
    /// Selected season id; `None` = all time.
    value: Option<String>,
    /// Fired on change. `None` = all time; `Some(id)` = season id verbatim.
    onchange: EventHandler<Option<String>>,
    /// Optional `id` on the underlying `<select>`.
    #[props(default)]
    id: Option<String>,
    /// Optional visible label rendered above the control.
    #[props(default)]
    label: Option<String>,
) -> Element {
    let seasons = use_api::<Vec<Season>>("/api/public/seasons");
    let current = value.unwrap_or_default();
    let data = seasons.data.read();
    let list: Vec<Season> = data.as_ref().and_then(|d| d.clone()).unwrap_or_default();
    if list.is_empty() {
        return rsx! {};
    }
    rsx! {
        div { class: "season-select",
            if let Some(label) = label {
                Label { {label} }
            }
            select {
                class: "ui-field",
                id,
                value: "{current}",
                "aria-label": "Season",
                onchange: move |e| {
                    let v = e.value();
                    onchange.call(if v.is_empty() { None } else { Some(v) });
                },
                option { key: "all", value: "", "{ALL_TIME_LABEL}" }
                for s in list.iter() {
                    option {
                        key: "{s.id}",
                        value: "{s.id}",
                        if s.is_current { "{s.name} (current)" } else { "{s.name}" }
                    }
                }
            }
        }
    }
}
