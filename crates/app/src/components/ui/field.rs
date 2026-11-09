use dioxus::prelude::*;

pub const FIELD_CSS: &str = r#"
.ui-field { width: 100%; font-family: var(--font-body); font-size: var(--text-sm); font-weight: 500;
  background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-md);
  padding: var(--space-2) var(--space-3); color: var(--text); }
.ui-field::placeholder { color: var(--text-3); }
.ui-field:focus { border-color: var(--accent); }
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

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

/// Lowercase ASCII slug: runs of anything not alphanumeric become one `-`.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// `id` written onto the input. Ids come from data, never a counter, so they stay
/// stable across renders and navigation. With a built-in label the id is the
/// explicit `id`, else the field `name`, else `field-<label slug>`. Pass an
/// explicit id when two inputs on one page share a label and have no `name`.
fn input_dom_id(explicit: Option<&str>, label: Option<&str>, name: Option<&str>) -> Option<String> {
    if let Some(id) = non_blank(explicit) {
        return Some(id.to_string());
    }
    let label = non_blank(label)?;
    if let Some(name) = non_blank(name) {
        return Some(name.to_string());
    }
    let slug = slug(label);
    Some(if slug.is_empty() {
        "field".to_string()
    } else {
        format!("field-{slug}")
    })
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
    /// Optional `name` on the input.
    #[props(default)]
    name: Option<String>,
) -> Element {
    let show_label = label.as_ref().is_some_and(|text| !text.trim().is_empty());
    let label_text = label.clone().unwrap_or_default();
    let dom_id = input_dom_id(id.as_deref(), label.as_deref(), name.as_deref());
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
                    name,
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
                name,
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
    #[props(default)] id: Option<String>,
    #[props(default)] name: Option<String>,
) -> Element {
    rsx! {
        textarea {
            class: field_class(invalid),
            id,
            name,
            placeholder,
            value: "{value}",
            oninput: move |e| { if let Some(h) = &oninput { h.call(e); } },
        }
    }
}

#[component]
pub fn Select(
    #[props(default = false)] invalid: bool,
    #[props(default)] onchange: Option<EventHandler<FormEvent>>,
    #[props(default)] id: Option<String>,
    #[props(default)] name: Option<String>,
    children: Element,
) -> Element {
    rsx! {
        select {
            class: field_class(invalid),
            id,
            name,
            onchange: move |e| { if let Some(h) = &onchange { h.call(e); } },
            {children}
        }
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
        assert_eq!(input_dom_id(None, None, None), None);
        assert_eq!(input_dom_id(Some("  "), None, None), None);
        assert_eq!(input_dom_id(None, Some("   "), Some("email")), None);
        assert_eq!(
            input_dom_id(Some("email"), None, None).as_deref(),
            Some("email")
        );
    }

    #[test]
    fn labelled_input_id_comes_from_data() {
        // Explicit id wins over name and label.
        assert_eq!(
            input_dom_id(Some(" member-42-email "), Some("Email"), Some("email")).as_deref(),
            Some("member-42-email")
        );
        // Then the field name.
        assert_eq!(
            input_dom_id(None, Some("Email"), Some("contact-email")).as_deref(),
            Some("contact-email")
        );
        // Then a slug of the label text.
        assert_eq!(
            input_dom_id(None, Some("Display name"), None).as_deref(),
            Some("field-display-name")
        );
        assert_eq!(
            input_dom_id(None, Some("  Discord / Battle.net tag! "), Some(" ")).as_deref(),
            Some("field-discord-battle-net-tag")
        );
        assert_eq!(
            input_dom_id(None, Some("!!"), None).as_deref(),
            Some("field")
        );
    }

    #[test]
    fn labelled_input_id_is_stable_across_renders() {
        let first = input_dom_id(None, Some("Org name"), None);
        assert_eq!(first, input_dom_id(None, Some("Org name"), None));
        assert_eq!(first.as_deref(), Some("field-org-name"));
        assert_ne!(first, input_dom_id(None, Some("Org tag"), None));
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
