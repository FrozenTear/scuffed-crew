use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use scuffed_types::{
    MeResponse, OkResponse, OrgRole, SetupRequest, SetupStatusResponse, UserInfo,
    homepage_preset_by_id, homepage_presets,
};

use crate::pages::display_name::{
    DisplayNameAudience, DisplayNameField, name_field_after_edit, settle_name_field,
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
    color: var(--text-subtle);
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
    color: var(--text-3);
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
    color: var(--text-subtle);
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
    color: var(--loss);
    font-size: 0.85rem;
    margin-bottom: 1rem;
}
/* No text: out of the layout, still in the tree so the next message is
   announced. */
.setup-error:empty {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
}
.setup-card button[type="submit"] {
    width: 100%;
    margin-top: 0.5rem;
}
.setup-pack-meta {
    font-family: var(--font-mono);
    font-size: 0.72rem;
    color: var(--text-3);
    margin: 0.35rem 0 0;
    letter-spacing: 0.02em;
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
    let mut name_error = use_signal(|| Option::<String>::None);
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
        if let Some(Some(s)) = status.value()()
            && !s.needs_setup
        {
            nav.replace(Route::Login {});
        }
    });

    let on_submit = move |evt: Event<FormData>| {
        evt.prevent_default();
        error.set(None);
        name_error.set(None);
        let typed_at_submit = username();
        let user = typed_at_submit.trim().to_string();
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
                    // Logged once here, whichever line shows it (Vision nit 3,
                    // round 4 on 748); the mapping below is pure.
                    crate::util::log_client_error("Create admin account", &e);
                    let settled = settle_name_field(
                        &typed_at_submit,
                        &username(),
                        &e,
                        DisplayNameAudience::Editor,
                    );
                    username.set(settled.value);
                    if let Some(message) = settled.error {
                        name_error.set(Some(message.to_string()));
                    } else if settled.keep_existing_failure {
                        error.set(Some(crate::client_error::user_message(&e)));
                    }
                    submitting.set(false);
                }
            }
        });
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-setup-css", css: CSS }
        div { class: "setup-page",
            div { class: "setup-card",
                h1 { "Create admin account" }
                p { class: "lead",
                    "First-time setup for your clan platform. Create the admin account, name the org, and pick a homepage starter."
                }
                // Always rendered so a new message is announced; only its text
                // changes, and the empty paragraph is out of the layout.
                p { class: "setup-error", role: "alert", "{error().unwrap_or_default()}" }
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
                        label { r#for: "setup-preset", "Homepage identity pack" }
                        select {
                            id: "setup-preset",
                            value: "{homepage_preset}",
                            onchange: move |e| homepage_preset.set(e.value()),
                            for p in homepage_presets() {
                                option { value: "{p.id}", "{p.name}" }
                            }
                        }
                        {
                            let meta = homepage_preset_by_id(&homepage_preset()).map(|p| {
                                (
                                    p.description.to_string(),
                                    p.suggested_shell.as_str(),
                                    p.suggested_skin.as_str(),
                                )
                            });
                            if let Some((desc, shell, skin)) = meta {
                                rsx! {
                                    p { class: "setup-hint", "{desc}" }
                                    p { class: "setup-pack-meta",
                                        "shell={shell} · skin={skin}"
                                    }
                                }
                            } else {
                                rsx! {
                                    p { class: "setup-hint",
                                        "You can change everything later in Admin → Settings."
                                    }
                                }
                            }
                        }
                        p { class: "setup-hint",
                            "Packs set homepage layout and starter copy. Edit anytime in Admin → Settings."
                        }
                    }
                    DisplayNameField {
                        id: "setup-user".to_string(),
                        label: "Username".to_string(),
                        value: username(),
                        error: name_error(),
                        autocomplete: "username".to_string(),
                        field_class: "setup-field".to_string(),
                        oninput: move |e: FormEvent| {
                            let (next, cleared) = name_field_after_edit(e.value());
                            username.set(next);
                            name_error.set(cleared.map(str::to_string));
                        },
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

#[cfg(test)]
mod tests {
    /// Vision nit 3, round 4 on 748: a failed first-admin setup logs its raw
    /// error once, at the top of the failure arm, whichever line (the name
    /// field or the form error) then shows it, and the mapping stays pure.
    #[test]
    fn a_failed_setup_logs_once() {
        let src = include_str!("setup.rs");
        let prod = src.split("mod tests").next().expect("production source");
        assert_eq!(prod.matches("log_client_error(").count(), 1);
        assert!(!prod.contains("logged_message("), "a second log");
        assert!(!prod.contains("with_lead("), "a second log");
        let arm = prod.find("Err(e) => {").expect("failure arm");
        let log = prod.find("log_client_error(").expect("log");
        let settle = prod.find("settle_name_field(\n").expect("settle");
        let shown = prod.find("user_message(&e)").expect("form error");
        assert!(arm < log && log < settle && settle < shown);
    }
}
