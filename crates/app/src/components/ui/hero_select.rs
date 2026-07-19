use crate::components::ui::Label;
use dioxus::prelude::*;
use scuffed_types::HEROES;

/// Display label for the sentinel "no hero filter" option.
pub const ALL_HEROES_LABEL: &str = "All heroes";

/// Layout CSS for the label + select stack. The `<select>` itself reuses the
/// shared `.ui-field` class from `field::FIELD_CSS`, so styling stays
/// consistent with every other form control.
pub const HERO_SELECT_CSS: &str = r#"
.hero-select { display: flex; flex-direction: column; gap: var(--space-1); }
"#;

/// Ordered option list backing [`HeroSelect`]: the "All heroes" sentinel first,
/// then every canonical hero from [`scuffed_types::HEROES`] verbatim.
///
/// Each entry is `(value, label)`. The sentinel carries an **empty** `value`
/// (heroes are never empty), which the selection model maps to `None`. Every
/// other `value` is the exact canonical `HEROES` string, so the emitted
/// `Some(name)` can be forwarded straight to the API as `?hero=`.
fn hero_options() -> Vec<(&'static str, &'static str)> {
    let mut opts = Vec::with_capacity(HEROES.len() + 1);
    opts.push(("", ALL_HEROES_LABEL));
    for &hero in HEROES {
        opts.push((hero, hero));
    }
    opts
}

/// Shared hero picker. Emits `None` for "All heroes" (no filter) or
/// `Some(canonical_name)` for a specific hero. Drop-in for any page that needs
/// a hero filter (leaderboards, roster, profile — later waves).
#[component]
pub fn HeroSelect(
    /// Current selection. `None` = "All heroes" (no filter); `Some(name)` must
    /// be an exact canonical [`HEROES`] string.
    value: Option<String>,
    /// Fired on change. `None` = "All heroes"; `Some(name)` = canonical
    /// [`HEROES`] string, emitted verbatim.
    onchange: EventHandler<Option<String>>,
    /// Optional `id` on the underlying `<select>` (e.g. for label `for=`).
    #[props(default)]
    id: Option<String>,
    /// Optional visible label rendered above the control.
    #[props(default)]
    label: Option<String>,
    /// Disable the control.
    #[props(default = false)]
    disabled: bool,
) -> Element {
    let current = value.unwrap_or_default();
    rsx! {
        div { class: "hero-select",
            if let Some(label) = label {
                Label { {label} }
            }
            select {
                class: "ui-field",
                id,
                disabled,
                value: "{current}",
                onchange: move |e| {
                    let v = e.value();
                    onchange.call(if v.is_empty() { None } else { Some(v) });
                },
                for (val , text) in hero_options() {
                    option { key: "{val}", value: val, "{text}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_lead_with_all_heroes_sentinel() {
        let opts = hero_options();
        assert_eq!(opts.len(), HEROES.len() + 1);
        assert_eq!(opts[0], ("", ALL_HEROES_LABEL));
    }

    #[test]
    fn hero_options_are_canonical_verbatim() {
        let opts = hero_options();
        // Every non-sentinel entry maps value == label == canonical HEROES string.
        for (i, &hero) in HEROES.iter().enumerate() {
            assert_eq!(opts[i + 1], (hero, hero));
        }
    }
}
