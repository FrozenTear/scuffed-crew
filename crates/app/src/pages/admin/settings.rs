use dioxus::prelude::*;

use scuffed_api_client::ApiClient;
use scuffed_types::{SiteSettings, api::UpdateSettingsRequest};
use crate::components::{Toast, use_toast};

#[component]
pub fn AdminSettings() -> Element {
    let mut toast = use_toast();
    let mut saving = use_signal(|| false);

    let mut org_name = use_signal(String::new);
    let mut site_description = use_signal(String::new);
    let mut recruitment_open = use_signal(|| false);
    let mut recruitment_message = use_signal(String::new);
    let mut min_age = use_signal(|| "16".to_string());
    let mut forum_backend = use_signal(|| "local".to_string());

    let _settings = use_resource(move || async move {
        match ApiClient::web().fetch::<SiteSettings>("/api/settings").await {
            Ok(s) => {
                org_name.set(s.org_name);
                site_description.set(s.site_description);
                recruitment_open.set(s.recruitment_open);
                recruitment_message.set(s.recruitment_message);
                min_age.set(s.min_age.to_string());
                forum_backend.set(s.forum_backend);
                Some(true)
            }
            Err(_) => None,
        }
    });

    let on_save = move |_| {
        let age: u32 = min_age().trim().parse().unwrap_or(16);
        let body = UpdateSettingsRequest {
            org_name: Some(org_name().trim().to_string()),
            site_description: Some(site_description().trim().to_string()),
            recruitment_open: Some(recruitment_open()),
            recruitment_message: Some(recruitment_message().trim().to_string()),
            min_age: Some(age),
            forum_backend: Some(forum_backend()),
        };

        saving.set(true);
        spawn(async move {
            let client = ApiClient::web();
            match client.put_json::<_, SiteSettings>("/api/settings", &body).await {
                Ok(_) => {
                    toast.show(Toast::success("Settings saved."));
                }
                Err(e) => {
                    toast.show(Toast::error(format!("Failed to save settings: {e}")));
                }
            }
            saving.set(false);
        });
    };

    rsx! {

        div { class: "admin-toolbar",
            h1 { "Settings" }
            button {
                class: "btn-add",
                disabled: saving(),
                onclick: on_save,
                if saving() { "Saving..." } else { "Save Settings" }
            }
        }

        div { class: "form-section",
            h2 { "Organization" }
            div { class: "form-inline",
                div { class: "form-field",
                    label { class: "form-label", "Organization Name" }
                    input {
                        class: "form-input",
                        r#type: "text",
                        value: "{org_name}",
                        oninput: move |e| org_name.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "form-label", "Site Description" }
                    textarea {
                        class: "form-textarea",
                        value: "{site_description}",
                        oninput: move |e| site_description.set(e.value()),
                    }
                }
            }
        }

        div { class: "form-section",
            h2 { "Recruitment" }
            div { class: "form-inline",
                div { class: "form-field",
                    div { class: "form-checkbox-row",
                        input {
                            r#type: "checkbox",
                            checked: recruitment_open(),
                            onchange: move |e| recruitment_open.set(e.checked()),
                        }
                        label { class: "form-label", "Recruitment Open" }
                    }
                }
                div { class: "form-field",
                    label { class: "form-label", "Recruitment Message" }
                    textarea {
                        class: "form-textarea",
                        value: "{recruitment_message}",
                        oninput: move |e| recruitment_message.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "form-label", "Minimum Age" }
                    input {
                        class: "form-input",
                        r#type: "number",
                        min: "0",
                        value: "{min_age}",
                        oninput: move |e| min_age.set(e.value()),
                    }
                }
            }
        }

        div { class: "form-section",
            h2 { "Forum" }
            div { class: "form-inline",
                div { class: "form-field",
                    label { class: "form-label", "Forum Backend" }
                    select {
                        class: "form-input",
                        value: "{forum_backend}",
                        onchange: move |e| forum_backend.set(e.value()),
                        option { value: "local", "Local (SurrealDB)" }
                        option { value: "nostr", "Nostr (Relay)" }
                    }
                    p {
                        style: "font-size: 0.75rem; color: var(--text-muted); margin-top: 0.35rem;",
                        "Controls where forum data is stored. \"Local\" uses the database. \"Nostr\" uses the relay (requires relay setup)."
                    }
                }
            }
        }
    }
}
