use dioxus::prelude::*;
pub const CARD_CSS: &str = r#"
.ui-card { background: var(--surface); border: 1px solid var(--border);
  border-radius: var(--radius-lg); padding: var(--space-4); }
.ui-card--accent { border-left: 2px solid var(--accent); }
"#;
#[component]
pub fn Card(#[props(default = false)] accent_edge: bool, children: Element) -> Element {
    let class = if accent_edge {
        "ui-card ui-card--accent"
    } else {
        "ui-card"
    };
    rsx! { div { class: "{class}", {children} } }
}
