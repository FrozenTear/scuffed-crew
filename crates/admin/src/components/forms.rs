use leptos::prelude::*;

/// Text input field for admin forms.
#[component]
pub fn FormField(
    label: &'static str,
    value: RwSignal<String>,
    #[prop(default = "text")] input_type: &'static str,
    #[prop(optional)] placeholder: &'static str,
) -> impl IntoView {
    view! {
        <div class="form-group">
            <label class="form-label">{label}</label>
            <input
                class="form-input"
                type=input_type
                placeholder=placeholder
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
        <div class="form-group">
            <label class="form-label">{label}</label>
            <select
                class="form-input"
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

/// Checkbox field for admin forms.
#[component]
pub fn CheckboxField(label: &'static str, value: RwSignal<bool>) -> impl IntoView {
    view! {
        <div class="form-group">
            <label class="checkbox-label">
                <input
                    type="checkbox"
                    prop:checked=move || value.get()
                    on:change=move |ev| {
                        value.set(event_target_checked(&ev));
                    }
                />
                <span>{label}</span>
            </label>
        </div>
    }
}

pub fn event_target_checked(ev: &leptos::ev::Event) -> bool {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|el| el.checked())
        .unwrap_or(false)
}

/// Textarea field for admin forms.
#[component]
pub fn TextAreaField(
    label: &'static str,
    value: RwSignal<String>,
    #[prop(default = 4)] rows: u32,
    #[prop(optional)] placeholder: &'static str,
) -> impl IntoView {
    view! {
        <div class="form-group">
            <label class="form-label">{label}</label>
            <textarea
                class="form-input"
                rows=rows
                placeholder=placeholder
                prop:value=move || value.get()
                on:input=move |ev| value.set(event_target_value(&ev))
            />
        </div>
    }
}
