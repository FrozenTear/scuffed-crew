use dioxus::prelude::*;
pub const LABEL_CSS: &str = r#"
.ui-label { font-family: var(--font-mono); font-weight: 500; font-size: var(--text-xs);
  letter-spacing: .08em; text-transform: uppercase; color: var(--text-3); }
"#;
#[component]
pub fn Label(children: Element) -> Element {
    rsx! { span { class: "ui-label", {children} } }
}
