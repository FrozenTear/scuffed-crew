use dioxus::prelude::*;
pub const FIELD_CSS: &str = r#"
.ui-field { width: 100%; font-family: var(--font-body); font-size: var(--text-sm); font-weight: 500;
  background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-md);
  padding: var(--space-2) var(--space-3); color: var(--text); }
.ui-field::placeholder { color: var(--text-3); }
.ui-field:focus { outline: none; border-color: var(--accent); }
.ui-field--invalid { border-color: var(--danger); }
textarea.ui-field { resize: vertical; min-height: 80px; }
"#;

fn field_class(invalid: bool) -> &'static str {
    if invalid {
        "ui-field ui-field--invalid"
    } else {
        "ui-field"
    }
}

#[component]
pub fn Input(
    value: String,
    #[props(default)] placeholder: String,
    #[props(default = false)] invalid: bool,
    #[props(default = false)] disabled: bool,
    #[props(default)] oninput: Option<EventHandler<FormEvent>>,
) -> Element {
    rsx! {
        input { class: field_class(invalid), value, placeholder, disabled,
            oninput: move |e| { if let Some(h) = &oninput { h.call(e); } } }
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
}
