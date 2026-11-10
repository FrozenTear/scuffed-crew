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

/// `id` written onto the input: the explicit `id` when it is non-blank, else
/// none. There is no fallback to `name` or a label slug, because two inputs that
/// share a label or a name (or a name with spaces) would collide or be invalid.
/// An `id` should be built from data (record id plus field name), never a
/// global counter, so it stays stable across renders. A labelled input does
/// not need one: it is nested inside its `<label>`, which links it either way.
fn input_dom_id(explicit: Option<&str>) -> Option<String> {
    non_blank(explicit).map(str::to_string)
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
    /// Input `id`. Omitted when blank. Build it from data (record id plus
    /// field name) so it is unique on the page.
    #[props(default)]
    id: Option<String>,
    /// Visible label rendered above the input. The input is nested inside the
    /// `<label>`, so it is linked with or without an `id`; when an `id` is set
    /// the label also carries a matching `for`.
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
    let dom_id = input_dom_id(id.as_deref());
    let aria_invalid_attr = aria_bool(resolved_aria_invalid(invalid, aria_invalid));
    let aria_required_attr = aria_bool(aria_required);
    let class = field_class(invalid);

    if show_label {
        rsx! {
            label { class: "ui-field-wrap", r#for: dom_id.clone(),
                span { class: "ui-field-label", "{label_text}" }
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
    fn input_id_is_only_the_explicit_id() {
        assert_eq!(input_dom_id(None), None);
        assert_eq!(input_dom_id(Some("  ")), None);
        assert_eq!(input_dom_id(Some("email")).as_deref(), Some("email"));
        assert_eq!(
            input_dom_id(Some(" member-42-email ")).as_deref(),
            Some("member-42-email")
        );
    }

    #[test]
    fn explicit_input_id_is_stable_across_renders() {
        let first = input_dom_id(Some("org-7-name"));
        assert_eq!(first, input_dom_id(Some("org-7-name")));
        assert_ne!(first, input_dom_id(Some("org-7-tag")));
    }

    /// The `<input>` sits between `<label` and `</label>`.
    fn assert_input_inside_label(html: &str) {
        let open = html.find("<label").expect("label");
        let input = html.find("<input").expect("input");
        let close = html.find("</label>").expect("label close");
        assert!(open < input && input < close, "{html}");
    }

    fn render(app: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn labelled_input_uses_the_explicit_id_for_label_and_input() {
        fn view() -> Element {
            rsx! {
                Input {
                    value: String::new(),
                    id: "member-42-email".to_string(),
                    name: "email".to_string(),
                    label: "Email".to_string(),
                }
            }
        }
        let html = render(view);
        assert!(html.contains(r#"for="member-42-email""#), "{html}");
        assert!(html.contains(r#"id="member-42-email""#), "{html}");
        assert!(!html.contains(r#"id="email""#), "{html}");
        assert!(!html.contains("field-email"), "{html}");
        assert_input_inside_label(&html);
    }

    #[test]
    fn unlabelled_input_without_id_gets_no_id() {
        fn view() -> Element {
            rsx! {
                Input {
                    value: String::new(),
                    name: "contact email".to_string(),
                    aria_label: "Email".to_string(),
                }
            }
        }
        let html = render(view);
        assert!(!html.contains(" id="), "{html}");
        assert!(!html.contains(" for="), "{html}");
    }

    #[test]
    fn labelled_input_without_id_is_linked_by_nesting() {
        fn view() -> Element {
            rsx! {
                Input {
                    value: String::new(),
                    name: "email".to_string(),
                    label: "Email".to_string(),
                }
            }
        }
        fn blank_id() -> Element {
            rsx! {
                Input {
                    value: String::new(),
                    id: "   ".to_string(),
                    label: "Email".to_string(),
                }
            }
        }
        for html in [render(view), render(blank_id)] {
            assert_input_inside_label(&html);
            assert!(html.contains(">Email</span>"), "{html}");
            assert!(!html.contains(" id="), "{html}");
            assert!(!html.contains(" for="), "{html}");
        }
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
