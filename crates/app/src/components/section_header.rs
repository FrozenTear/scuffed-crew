use dioxus::prelude::*;

#[component]
pub fn SectionHeader(
    label: &'static str,
    title: &'static str,
    #[props(default = "purple")] color: &'static str,
    #[props(default)] description: Option<&'static str>,
) -> Element {
    let label_class = format!("sec-label sec-label-{color}");

    rsx! {
        div {
            div { class: "{label_class}", "{label}" }
            h2 { class: "sec-title", "{title}" }
            if let Some(desc) = description {
                p { class: "sec-desc", "{desc}" }
            }
        }
    }
}
