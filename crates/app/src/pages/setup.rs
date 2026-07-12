use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use scuffed_types::{
    MeResponse, OkResponse, OrgRole, SetupRequest, SetupStatusResponse, UserInfo,
    homepage_presets,
};

use crate::routes::Route;
use crate::state::auth::{AuthState, use_auth};

const CSS: &str = r#"
.setup-page {
    min-height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 2rem;
    background: var(--bg);
}
.setup-card {
    width: 100%;
    max-width: 480px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 2rem;
}
.setup-field select {
    background: var(--bg);
    border: 1px solid var(--border);
    color: var(--text);
    padding: 0.6rem 0.75rem;
    border-radius: 6px;
    font-size: 1rem;
}
.setup-hint {
    color: var(--text-3);
    font-size: 0.8rem;
    margin: 0.25rem 0 0;
}
.setup-card h1 {
    font-family: var(--font-head);
    font-size: 1.5rem;
    color: var(--text);
    margin: 0 0 0.5rem;
}
.setup-card p.lead {
    color: var(--text-2);
    font-size: 0.9rem;
    margin: 0 0 1.5rem;
}
.setup-field {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-bottom: 1rem;
}
.setup-field label {
    font-size: 0.75rem;
    color: var(--text-3);
    text-transform: uppercase;
    letter-spacing: 0.04em;
}
.setup-field input {
    background: var(--bg);
    border: 1px solid var(--border);
    color: var(--text);
    padding: 0.6rem 0.75rem;
    border-radius: 6px;
    font-size: 1rem;
}
.setup-error {
    color: var(--danger, #d63031);
    font-size: 0.85rem;
    margin-bottom: 1rem;
}
.setup-card button[type="submit"] {
    width: 100%;
    margin-top: 0.5rem;
}
"#;

fn me_to_user_info(me: &MeResponse) -> UserInfo {
    let role = me.member.as_ref().and_then(|m| match m.org_role.as_str() {
        "admin" => Some(OrgRole::Admin),
        "officer" => Some(OrgRole::Officer),
        "member" => Some(OrgRole::Member),
        "recruit" => Some(OrgRole::Recruit),
        _ => None,
    });
    UserInfo {
        id: me.user.id.clone(),
        username: me
            .member
            .as_ref()
            .map(|m| m.display_name.clone())
            .unwrap_or_else(|| me.user.username.clone()),
        avatar_url: me.user.avatar_url.clone(),
        role,
    }
}

#[component]
pub fn Setup() -> Element {
    let mut username = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut confirm = use_signal(String::new);
    let mut org_name = use_signal(|| "My Clan".to_string());
    let mut homepage_preset = use_signal(|| "neutral".to_string());
    let mut error = use_signal(|| Option::<String>::None);
    let mut submitting = use_signal(|| false);
    let mut auth = use_auth();
    let nav = use_navigator();

    let status = use_resource(|| async move {
        ApiClient::web()
            .fetch::<SetupStatusResponse>("/api/auth/setup-status")
            .await
            .ok()
    });

    use_effect(move || {
        if let Some(Some(s)) = status.value()() {
            if !s.needs_setup {
                nav.replace(Route::Login {});
            }
        }
    });

    let on_submit = move |evt: Event<FormData>| {
        evt.prevent_default();
        error.set(None);
        let user = username().trim().to_string();
        let pass = password();
        let conf = confirm();
        if user.is_empty() {
            error.set(Some("Username is required".into()));
            return;
        }
        if pass.len() < 12 {
            error.set(Some("Password must be at least 12 characters".into()));
            return;
        }
        if pass != conf {
            error.set(Some("Passwords do not match".into()));
            return;
        }
        submitting.set(true);
        let org = org_name().trim().to_string();
        let preset = homepage_preset();
        spawn(async move {
            let client = ApiClient::web();
            let body = SetupRequest {
                username: user,
                password: pass,
                org_name: if org.is_empty() { None } else { Some(org) },
                homepage_preset: Some(preset),
            };
            match client
                .post_json::<_, OkResponse>("/api/auth/setup", &body)
                .await
            {
                Ok(_) => {
                    if let Ok(me) = client.get_me().await {
                        auth.set(AuthState {
                            user: Some(me_to_user_info(&me)),
                            loading: false,
                        });
                    }
                    nav.replace(Route::AdminDashboard {});
                }
                Err(e) => {
                    error.set(Some(e.to_string()));
                    submitting.set(false);
                }
            }
        });
    };

    rsx! {
        style { {CSS} }
        div { class: "setup-page",
            div { class: "setup-card",
                h1 { "Create admin account" }
                p { class: "lead",
                    "First-time setup for your clan platform. Create the admin account, name the org, and pick a homepage starter."
                }
                if let Some(err) = error() {
                    p { class: "setup-error", "{err}" }
                }
                form { onsubmit: on_submit,
                    div { class: "setup-field",
                        label { r#for: "setup-org", "Clan / org name" }
                        input {
                            id: "setup-org",
                            r#type: "text",
                            value: "{org_name}",
                            oninput: move |e| org_name.set(e.value()),
                        }
                    }
                    div { class: "setup-field",
                        label { r#for: "setup-preset", "Homepage template" }
                        select {
                            id: "setup-preset",
                            value: "{homepage_preset}",
                            onchange: move |e| homepage_preset.set(e.value()),
                            for p in homepage_presets() {
                                option { value: "{p.id}", "{p.name}" }
                            }
                        }
                        p { class: "setup-hint",
                            "You can change copy, sections, and brand later in Admin → Settings."
                        }
                    }
                    div { class: "setup-field",
                        label { r#for: "setup-user", "Username" }
                        input {
                            id: "setup-user",
                            r#type: "text",
                            autocomplete: "username",
                            value: "{username}",
                            oninput: move |e| username.set(e.value()),
                        }
                    }
                    div { class: "setup-field",
                        label { r#for: "setup-pass", "Password (min 12 characters)" }
                        input {
                            id: "setup-pass",
                            r#type: "password",
                            autocomplete: "new-password",
                            value: "{password}",
                            oninput: move |e| password.set(e.value()),
                        }
                    }
                    div { class: "setup-field",
                        label { r#for: "setup-confirm", "Confirm password" }
                        input {
                            id: "setup-confirm",
                            r#type: "password",
                            autocomplete: "new-password",
                            value: "{confirm}",
                            oninput: move |e| confirm.set(e.value()),
                        }
                    }
                    button {
                        class: "ui-btn ui-btn--primary ui-btn--md",
                        r#type: "submit",
                        disabled: submitting(),
                        if submitting() { "Creating…" } else { "Create admin account" }
                    }
                }
            }
        }
    }
}
