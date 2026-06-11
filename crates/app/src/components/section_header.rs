use dioxus::prelude::*;

const SECTION_HEADER_CSS: &str = r#"
.sec-label { font-family: var(--font-mono); font-size: var(--text-xs); text-transform: uppercase; letter-spacing: 0.08em; margin-bottom: var(--space-2); color: var(--accent); }
.sec-label-accent { color: var(--accent); }
.sec-label-purple { color: var(--accent); }
.sec-label-red { color: var(--accent); }
.sec-label-blue { color: var(--accent); }
.sec-title { font-family: var(--font-head); font-size: var(--text-3xl); color: var(--text); letter-spacing: 3px; margin: 0 0 var(--space-2); }
.sec-desc { color: var(--text-2); max-width: 600px; line-height: 1.7; }
"#;

#[component]
pub fn SectionHeader(
    label: &'static str,
    title: &'static str,
    #[props(default = "accent")] color: &'static str,
    #[props(default)] description: Option<&'static str>,
) -> Element {
    let label_class = format!("sec-label sec-label-{color}");

    rsx! {
        style { {SECTION_HEADER_CSS} }
        div {
            div { class: "{label_class}", "{label}" }
            h2 { class: "sec-title", "{title}" }
            if let Some(desc) = description {
                p { class: "sec-desc", "{desc}" }
            }
        }
    }
}
