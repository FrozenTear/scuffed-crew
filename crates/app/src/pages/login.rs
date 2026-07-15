use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use scuffed_types::{AuthProvidersResponse, LocalLoginRequest, OkResponse, SetupStatusResponse};

use crate::routes::Route;
use crate::state::auth::{AuthState, use_auth};
use scuffed_types::{MeResponse, OrgRole, UserInfo};

const CSS: &str = r#"
.login-page {
    min-height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 2rem;
    background: var(--bg);
}
.login-card {
    width: 100%;
    max-width: 420px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 2rem;
}
.login-card h1 {
    font-family: var(--font-head);
    font-size: 1.5rem;
    color: var(--text);
    margin: 0 0 0.5rem;
}
.login-card p.lead {
    color: var(--text-2);
    font-size: 0.9rem;
    margin: 0 0 1.5rem;
}
.login-field {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-bottom: 1rem;
}
.login-field label {
    font-size: 0.75rem;
    color: var(--text-3);
    text-transform: uppercase;
    letter-spacing: 0.04em;
}
.login-field input {
    background: var(--bg);
    border: 1px solid var(--border);
    color: var(--text);
    padding: 0.6rem 0.75rem;
    border-radius: 6px;
    font-size: 1rem;
}
.login-error {
    color: var(--danger);
    font-size: 0.85rem;
    margin-bottom: 1rem;
}
.login-oauth {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-top: 1.25rem;
}
.login-oauth a {
    text-align: center;
    padding: 0.55rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text-2);
    font-size: 0.9rem;
}
.login-oauth a:hover {
    color: var(--text);
    border-color: var(--accent);
}
.login-card button[type="submit"] {
    width: 100%;
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
pub fn Login() -> Element {
    let mut username = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut error = use_signal(|| Option::<String>::None);
    let mut submitting = use_signal(|| false);
    let mut auth = use_auth();
    let nav = use_navigator();

    let setup = use_resource(|| async move {
        ApiClient::web()
            .fetch::<SetupStatusResponse>("/api/auth/setup-status")
            .await
            .ok()
    });

    let providers = use_resource(|| async move {
        ApiClient::web()
            .fetch::<AuthProvidersResponse>("/api/auth/providers")
            .await
            .ok()
    });

    use_effect(move || {
        if let Some(Some(s)) = setup.value()()
            && s.needs_setup
        {
            nav.replace(Route::Setup {});
        }
    });

    let on_submit = move |evt: Event<FormData>| {
        evt.prevent_default();
        error.set(None);
        let user = username();
        let pass = password();
        if user.trim().is_empty() || pass.is_empty() {
            error.set(Some("Enter username and password".into()));
            return;
        }
        submitting.set(true);
        spawn(async move {
            let client = ApiClient::web();
            let body = LocalLoginRequest {
                username: user,
                password: pass,
            };
            match client
                .post_json::<_, OkResponse>("/api/auth/local/login", &body)
                .await
            {
                Ok(_) => {
                    if let Ok(me) = client.get_me().await {
                        auth.set(AuthState {
                            user: Some(me_to_user_info(&me)),
                            loading: false,
                        });
                    }
                    nav.replace(Route::Home {});
                }
                Err(_) => {
                    error.set(Some("Invalid username or password".into()));
                    submitting.set(false);
                }
            }
        });
    };

    let p = providers.value()().flatten();
    let show_local = p.as_ref().map(|x| x.local).unwrap_or(true);
    let show_discord = p.as_ref().map(|x| x.discord).unwrap_or(false);
    let show_google = p.as_ref().map(|x| x.google).unwrap_or(false);

    rsx! {
        style { {CSS} }
        div { class: "login-page",
            div { class: "login-card",
                h1 { "Sign in" }
                p { class: "lead", "Sign in to continue." }
                if let Some(err) = error() {
                    p { class: "login-error", "{err}" }
                }
                if show_local {
                    form { onsubmit: on_submit,
                        div { class: "login-field",
                            label { r#for: "login-user", "Username" }
                            input {
                                id: "login-user",
                                r#type: "text",
                                autocomplete: "username",
                                value: "{username}",
                                oninput: move |e| username.set(e.value()),
                            }
                        }
                        div { class: "login-field",
                            label { r#for: "login-pass", "Password" }
                            input {
                                id: "login-pass",
                                r#type: "password",
                                autocomplete: "current-password",
                                value: "{password}",
                                oninput: move |e| password.set(e.value()),
                            }
                        }
                        button {
                            class: "ui-btn ui-btn--primary ui-btn--md",
                            r#type: "submit",
                            disabled: submitting(),
                            if submitting() { "Signing in…" } else { "Sign in" }
                        }
                    }
                }
                if show_discord || show_google {
                    div { class: "login-oauth",
                        if show_discord {
                            a { href: "/api/auth/discord/login", "Sign in with Discord" }
                        }
                        if show_google {
                            a { href: "/api/auth/google/login", "Sign in with Google" }
                        }
                    }
                }
                if !show_local && !show_discord && !show_google {
                    p { class: "lead", "No login methods are configured." }
                    if cfg!(debug_assertions) {
                        a {
                            href: "/api/dev/login",
                            style: "color: var(--accent);",
                            "Dev login (in-memory only)"
                        }
                    }
                }
            }
        }
    }
}
