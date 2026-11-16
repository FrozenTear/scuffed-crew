use dioxus::prelude::*;

pub const FIELD_CSS: &str = r#"
.ui-field { width: 100%; font-family: var(--font-body); font-size: var(--text-sm); font-weight: 500;
  background: var(--surface-sunken); border: 1px solid var(--border); border-radius: var(--radius-control);
  padding: var(--space-2) var(--space-3); color: var(--text); }
.ui-field::placeholder { color: var(--text-subtle); }
.ui-field:focus { border-color: var(--accent-solid); }
.ui-field--invalid { border-color: var(--loss); }
textarea.ui-field { resize: vertical; min-height: 80px; }
.ui-field-wrap { display: flex; flex-direction: column; gap: 0.35rem; width: 100%; }
.ui-field-label { font-family: var(--font-body); font-size: var(--text-sm); font-weight: 600; color: var(--text-3); }
"#;

/// The redesign form field (the Apply frame): a 44px control with a strong
/// edge on `--surface`, a sentence-case label above it, a hint, and an error
/// line that is an icon plus text. On phone the control text is 16px, so the
/// browser does not zoom the page on focus. Add `field-input` to an `input`,
/// `select` or `textarea`; mark an invalid one with `aria-invalid="true"`.
pub const FORM_FIELD_CSS: &str = r#"
.field-label {
  display: block;
  min-height: 0;
  margin: 0;
  color: var(--text);
  font-family: var(--font-head);
  font-size: var(--text-sm);
  font-weight: var(--font-label-weight);
  line-height: var(--leading-ui);
}
.field-input {
  box-sizing: border-box;
  width: 100%;
  min-height: var(--hit-target-min);
  margin: 0;
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-control);
  background: var(--surface);
  color: var(--text);
  font-family: var(--font-body);
  font-size: var(--text-md);
  font-weight: var(--font-body-weight);
  line-height: var(--leading-body);
}
textarea.field-input {
  min-height: calc(var(--hit-target-min) * 3);
  padding: var(--space-3);
  resize: vertical;
}
.field-input::placeholder {
  color: var(--text-subtle);
  opacity: 1;
}
/* The browser's own clear button of a search field is drawn in a fixed
   colour that ignores the theme. Redraw it as a cross in the muted ink. */
.field-input[type="search"]::-webkit-search-cancel-button {
  -webkit-appearance: none;
  appearance: none;
  flex: 0 0 auto;
  width: var(--space-3);
  height: var(--space-3);
  margin-inline-start: var(--space-2);
  background: var(--text-3);
  clip-path: polygon(15% 0%, 50% 35%, 85% 0%, 100% 15%, 65% 50%, 100% 85%, 85% 100%, 50% 65%, 15% 100%, 0% 85%, 35% 50%, 0% 15%);
  cursor: pointer;
}
.field-input:focus { border-color: var(--accent-solid); }
.field-input[aria-invalid="true"] { border-color: var(--loss); }
.field-hint {
  margin: 0;
  color: var(--text-3);
  font-size: var(--text-sm);
  line-height: var(--leading-body);
}
.field-error {
  display: flex;
  align-items: flex-start;
  gap: var(--space-2);
  margin: 0;
  color: var(--loss);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-body);
}
/* No error: out of the layout, still in the tree so the next one is
   announced. */
.field-error:empty {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}
.field-error-icon {
  flex: 0 0 var(--space-4);
  width: var(--space-4);
  height: var(--space-4);
  margin-top: var(--space-1);
  border-radius: var(--radius-pill);
  background: var(--loss);
  color: var(--on-loss);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-family: var(--font-head);
  font-size: var(--text-xs);
  font-weight: var(--font-label-weight);
  line-height: 1;
}
.field-error-icon::before { content: "!"; }
@media (max-width: 820px) {
  .field-input { font-size: var(--text-lg); }
}
"#;

/// The error line of a field: an icon, then the text, so it does not rest on
/// colour. The icon is decorative; the text carries the message. Point the
/// control's `aria-describedby` at `id`. With `role` set (`"alert"`), keep the
/// line rendered with empty `text` so a later error is announced.
#[component]
pub fn FieldError(id: String, text: String, #[props(default)] role: Option<String>) -> Element {
    rsx! {
        p { class: "field-error", id, role,
            if !text.is_empty() {
                span { class: "field-error-icon", aria_hidden: "true" }
                "{text}"
            }
        }
    }
}

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
    fn form_field_is_44px_with_16px_text_on_phone() {
        use crate::components::ui::test_support::{
            assert_ink_clears_aa, assert_sizes_from_tokens, css_rule, has_decl,
        };
        assert_sizes_from_tokens(FORM_FIELD_CSS);
        let input = css_rule(FORM_FIELD_CSS, ".field-input {");
        for decl in [
            "min-height: var(--hit-target-min)",
            "border: 1px solid var(--border-strong)",
            "background: var(--surface)",
            "font-size: var(--text-md)",
        ] {
            assert!(has_decl(&input, decl), "missing {decl}: {input}");
        }
        let phone = FORM_FIELD_CSS
            .split_once("@media (max-width: 820px) {")
            .expect("phone block")
            .1;
        assert!(
            has_decl(
                &css_rule(phone, ".field-input {"),
                "font-size: var(--text-lg)"
            ),
            "{phone}"
        );
        // One placeholder ink for every field.
        let placeholder = css_rule(FORM_FIELD_CSS, ".field-input::placeholder {");
        assert!(
            has_decl(&placeholder, "color: var(--text-subtle)"),
            "{placeholder}"
        );
        // Labels are sentence case in the default ink, with no 44px box.
        let label = css_rule(FORM_FIELD_CSS, ".field-label {");
        for decl in ["display: block", "min-height: 0", "color: var(--text)"] {
            assert!(has_decl(&label, decl), "missing {decl}: {label}");
        }
        assert!(!label.contains("text-transform"), "{label}");
        assert!(!label.contains("letter-spacing"), "{label}");
        // Invalid is a loss border plus the icon and text line.
        assert!(
            FORM_FIELD_CSS
                .contains(".field-input[aria-invalid=\"true\"] { border-color: var(--loss); }")
        );
        assert!(has_decl(
            &css_rule(FORM_FIELD_CSS, ".field-error {"),
            "color: var(--loss)"
        ));
        assert_ink_clears_aa("--loss", "--surface", 4.5);
        assert_ink_clears_aa("--on-loss", "--loss", 4.5);
        assert_ink_clears_aa("--text", "--surface", 4.5);
        assert_ink_clears_aa("--text-3", "--surface", 4.5);
    }

    #[test]
    fn field_error_is_an_icon_then_text_and_stays_mounted_when_empty() {
        fn shown() -> Element {
            rsx! { FieldError { id: "apply-role-err", text: "Choose a role." } }
        }
        fn idle() -> Element {
            rsx! { FieldError { id: "wiki-1-err", text: "", role: "alert" } }
        }
        assert_eq!(
            render(shown),
            "<p class=\"field-error\" id=\"apply-role-err\"><span class=\"field-error-icon\" aria-hidden=\"true\"></span>Choose a role.</p>"
        );
        assert_eq!(
            render(idle),
            "<p class=\"field-error\" id=\"wiki-1-err\" role=\"alert\"></p>"
        );
    }

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
