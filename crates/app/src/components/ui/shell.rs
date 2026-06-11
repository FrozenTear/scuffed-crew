use dioxus::prelude::*;
pub const SHELL_CSS: &str = r#"
.ui-shell { max-width: 1100px; margin: 0 auto; padding: var(--space-6) var(--space-4) var(--space-12); }
"#;
#[component]
pub fn PageShell(children: Element) -> Element {
    rsx! { div { class: "ui-shell", {children} } }
}
