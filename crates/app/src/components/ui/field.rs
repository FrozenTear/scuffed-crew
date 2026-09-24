use std::sync::atomic::{AtomicU64, Ordering};

use dioxus::prelude::*;

static INPUT_IDS: AtomicU64 = AtomicU64::new(1);

pub const FIELD_CSS: &str = r#"
.ui-field { width: 100%; font-family: var(--font-body); font-size: var(--text-sm); font-weight: 500;
  background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-md);
  padding: var(--space-2) var(--space-3); color: var(--text); }
.ui-field::placeholder { color: var(--text-3); }
.ui-field:focus { outline: none; border-color: var(--accent); }
.ui-field--invalid { border-color: var(--danger); }
textarea.ui-field { resize: vertical; min-height: 80px; }
.ui-field-wrap { display: flex; flex-direction: column; gap: 0.35rem; width: 100%; }
.ui-field-label { font-family: var(--font-body); font-size: var(--text-sm); font-weight: 600; color: var(--text-2); }
"#;

fn field_class(invalid: bool) -> &'static str {
    if invalid {
        "ui-field ui-field--invalid"
    } else {
        "ui-field"
    }
}

/// `id` written onto the input. A built-in label always gets an id so `for` matches.
fn input_dom_id(explicit: Option<&str>, show_label: bool, generated: &str) -> Option<String> {
    let explicit = explicit
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if show_label {
        Some(explicit.unwrap_or_else(|| generated.to_string()))
    } else {
        explicit
    }
}

/// Explicit `aria_invalid` wins. Otherwise an invalid field exposes `true`.
fn resolved_aria_invalid(invalid: bool, explicit: Option<bool>) -> Option<bool> {
    explicit.or(invalid.then_some(true))
}

fn aria_bool(value: Option<bool>) -> Option<&'static str> {
    value.map(|on| if on { "true" } else { "false" })
}

#[component]
pub fn Input(
    value: String,
    #[props(default)] placeholder: String,
    #[props(default = false)] invalid: bool,
    #[props(default = false)] disabled: bool,
    #[props(default)] oninput: Option<EventHandler<FormEvent>>,
    /// Associates an external `<label for>`. Omitted when empty and no built-in label.
    #[props(default)]
    id: Option<String>,
    /// Visible label rendered above the input and wired with `for` / `id`.
    #[props(default)]
    label: Option<String>,
    #[props(default)] aria_label: Option<String>,
    #[props(default)] aria_describedby: Option<String>,
    #[props(default)] aria_invalid: Option<bool>,
    #[props(default)] aria_required: Option<bool>,
) -> Element {
    // Always run: hook order must not depend on whether a label was passed.
    let generated_id: String =
        use_hook(|| format!("ui-input-{}", INPUT_IDS.fetch_add(1, Ordering::Relaxed)));
    let show_label = label.as_ref().is_some_and(|text| !text.trim().is_empty());
    let label_text = label.clone().unwrap_or_default();
    let dom_id = input_dom_id(id.as_deref(), show_label, &generated_id);
    let aria_invalid_attr = aria_bool(resolved_aria_invalid(invalid, aria_invalid));
    let aria_required_attr = aria_bool(aria_required);
    let class = field_class(invalid);

    if show_label {
        let id_for = dom_id.clone().unwrap_or_default();
        rsx! {
            div { class: "ui-field-wrap",
                label { class: "ui-field-label", r#for: "{id_for}", "{label_text}" }
                input {
                    class: "{class}",
                    id: dom_id,
                    value,
                    placeholder,
                    disabled,
                    aria_label,
                    aria_describedby,
                    aria_invalid: aria_invalid_attr,
                    aria_required: aria_required_attr,
                    oninput: move |e| { if let Some(h) = &oninput { h.call(e); } },
                }
            }
        }
    } else {
        rsx! {
            input {
                class: "{class}",
                id: dom_id,
                value,
                placeholder,
                disabled,
                aria_label,
                aria_describedby,
                aria_invalid: aria_invalid_attr,
                aria_required: aria_required_attr,
                oninput: move |e| { if let Some(h) = &oninput { h.call(e); } },
            }
        }
    }
}

#[component]
pub fn Textarea(
    value: String,
    #[props(default)] placeholder: String,
    #[props(default = false)] invalid: bool,
    #[props(default)] oninput: Option<EventHandler<FormEvent>>,
) -> Element {
    rsx! {
        textarea { class: field_class(invalid), placeholder, value: "{value}",
            oninput: move |e| { if let Some(h) = &oninput { h.call(e); } } }
    }
}

#[component]
pub fn Select(
    #[props(default = false)] invalid: bool,
    #[props(default)] onchange: Option<EventHandler<FormEvent>>,
    children: Element,
) -> Element {
    rsx! {
        select { class: field_class(invalid),
            onchange: move |e| { if let Some(h) = &onchange { h.call(e); } }, {children} }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn field_class_reflects_invalid() {
        assert_eq!(field_class(false), "ui-field");
        assert_eq!(field_class(true), "ui-field ui-field--invalid");
    }

    #[test]
    fn input_id_stays_off_until_a_caller_associates_a_label() {
        assert_eq!(input_dom_id(None, false, "ui-input-1"), None);
        assert_eq!(input_dom_id(Some("  "), false, "ui-input-1"), None);
        assert_eq!(
            input_dom_id(Some("email"), false, "ui-input-1").as_deref(),
            Some("email")
        );
        assert_eq!(
            input_dom_id(None, true, "ui-input-4").as_deref(),
            Some("ui-input-4")
        );
        assert_eq!(
            input_dom_id(Some("email"), true, "ui-input-4").as_deref(),
            Some("email")
        );
    }

    #[test]
    fn aria_invalid_follows_the_invalid_flag_unless_overridden() {
        assert_eq!(resolved_aria_invalid(false, None), None);
        assert_eq!(resolved_aria_invalid(true, None), Some(true));
        assert_eq!(resolved_aria_invalid(true, Some(false)), Some(false));
        assert_eq!(aria_bool(Some(true)), Some("true"));
        assert_eq!(aria_bool(Some(false)), Some("false"));
        assert_eq!(aria_bool(None), None);
    }
}
