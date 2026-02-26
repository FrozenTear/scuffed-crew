use leptos::prelude::*;

/// Text input field for admin forms.
#[component]
pub fn FormField(
    label: &'static str,
    value: RwSignal<String>,
    #[prop(default = "text")] input_type: &'static str,
) -> impl IntoView {
    view! {
        <div>
            <label>{label}</label>
            <input
                type=input_type
                prop:value=move || value.get()
                on:input=move |ev| value.set(event_target_value(&ev))
            />
        </div>
    }
}

/// Select field for admin forms.
#[component]
pub fn SelectField(
    label: &'static str,
    value: RwSignal<String>,
    options: Vec<(&'static str, &'static str)>,
) -> impl IntoView {
    view! {
        <div>
            <label>{label}</label>
            <select
                prop:value=move || value.get()
                on:change=move |ev| value.set(event_target_value(&ev))
            >
                {options.into_iter().map(|(val, display)| {
                    view! { <option value=val>{display}</option> }
                }).collect_view()}
            </select>
        </div>
    }
}
