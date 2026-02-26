use leptos::prelude::*;

#[component]
pub fn SectionHeader(
    label: &'static str,
    title: &'static str,
    #[prop(default = "purple")]
    color: &'static str,
    #[prop(optional, into)]
    description: Option<&'static str>,
) -> impl IntoView {
    let label_class = format!("sec-label sec-label-{color}");

    view! {
        <div data-reveal="">
            <div class=label_class>{label}</div>
            <h2 class="sec-title">{title}</h2>
            {description.map(|desc| view! {
                <p class="sec-desc">{desc}</p>
            })}
        </div>
    }
}
