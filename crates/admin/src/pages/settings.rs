use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};

use scuffed_ui::components::button::{Button, ButtonVariant};
use scuffed_ui::components::toast::{use_toast, Toast};

use crate::api;
use crate::components::forms::{CheckboxField, FormField, TextAreaField};

#[derive(Debug, Clone, Deserialize)]
struct SiteSettings {
    #[allow(dead_code)]
    id: String,
    org_name: String,
    site_description: String,
    recruitment_open: bool,
    recruitment_message: String,
    min_age: u32,
}

#[derive(Serialize)]
struct UpdateSettingsBody {
    org_name: String,
    site_description: String,
    recruitment_open: bool,
    recruitment_message: String,
    min_age: u32,
}

#[component]
pub fn SettingsPage() -> impl IntoView {
    let toast = use_toast();

    let org_name = RwSignal::new(String::new());
    let site_description = RwSignal::new(String::new());
    let recruitment_open = RwSignal::new(true);
    let recruitment_message = RwSignal::new(String::new());
    let min_age = RwSignal::new(String::new());
    let loading = RwSignal::new(true);
    let saving = RwSignal::new(false);

    // Load settings on mount
    spawn_local(async move {
        match api::get::<SiteSettings>("/api/settings").await {
            Ok(s) => {
                org_name.set(s.org_name);
                site_description.set(s.site_description);
                recruitment_open.set(s.recruitment_open);
                recruitment_message.set(s.recruitment_message);
                min_age.set(s.min_age.to_string());
            }
            Err(e) => toast.show(Toast::error(format!("Failed to load settings: {e}"))),
        }
        loading.set(false);
    });

    let do_save = move || {
        let age = min_age.get().parse::<u32>().unwrap_or(16);
        saving.set(true);
        spawn_local(async move {
            let body = UpdateSettingsBody {
                org_name: org_name.get(),
                site_description: site_description.get(),
                recruitment_open: recruitment_open.get(),
                recruitment_message: recruitment_message.get(),
                min_age: age,
            };
            match api::put::<_, SiteSettings>("/api/settings", &body).await {
                Ok(_) => toast.show(Toast::success("Settings saved")),
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
            saving.set(false);
        });
    };

    view! {
        <h1>"Settings"</h1>
        {move || {
            if loading.get() {
                view! { <p class="empty-state">"Loading..."</p> }.into_any()
            } else {
                view! {
                    <div class="settings-form">
                        <div class="form-section">
                            <div class="form-section-title">"Organization"</div>
                            <div class="admin-form">
                                <FormField label="Organization Name" value=org_name placeholder="Your org name"/>
                                <TextAreaField label="Site Description" value=site_description rows=3 placeholder="Brief description of your organization"/>
                            </div>
                        </div>

                        <div class="form-section">
                            <div class="form-section-title">"Recruitment"</div>
                            <div class="admin-form">
                                <CheckboxField label="Recruitment Open" value=recruitment_open/>
                                <TextAreaField label="Recruitment Message" value=recruitment_message rows=3 placeholder="Message shown to potential applicants"/>
                                <FormField label="Minimum Age" value=min_age input_type="number"/>
                            </div>
                        </div>

                        <div style="display: flex; justify-content: flex-end; margin-top: 0.5rem;">
                            <Button
                                variant=ButtonVariant::Primary
                                on_click=Callback::new(move |_| do_save())
                            >
                                {move || if saving.get() { "Saving..." } else { "Save Settings" }}
                            </Button>
                        </div>
                    </div>
                }.into_any()
            }
        }}
    }
}
