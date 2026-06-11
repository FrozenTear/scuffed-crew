use dioxus::prelude::*;
pub const FEEDBACK_CSS: &str = r#"
.ui-empty { text-align: center; color: var(--text-2); padding: var(--space-8) var(--space-4); }
.ui-empty h3 { font-family: var(--font-head); color: var(--text); font-size: var(--text-lg); margin-bottom: var(--space-2); }
.ui-empty p { font-size: var(--text-sm); }
.ui-spinner { width: 18px; height: 18px; border: 2px solid var(--border);
  border-top-color: var(--accent); border-radius: 50%; animation: ui-spin .7s linear infinite; }
@keyframes ui-spin { to { transform: rotate(360deg); } }
"#;
#[component]
pub fn EmptyState(title: String, #[props(default)] message: Option<String>) -> Element {
    rsx! {
        div { class: "ui-empty",
            h3 { "{title}" }
            if let Some(msg) = message {
                p { "{msg}" }
            }
        }
    }
}
#[component]
pub fn Spinner() -> Element {
    rsx! { div { class: "ui-spinner" } }
}
