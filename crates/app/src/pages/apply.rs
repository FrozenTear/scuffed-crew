use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::ui::{BtnVariant, Button, Card, Pill, PillTone, Textarea};
use crate::components::{Toast, use_toast};
use crate::hooks::use_api;
use crate::routes::Route;
use crate::state::auth::use_auth;
use scuffed_api_client::ApiClient;
use scuffed_types::{Game, SiteSettings};

// Local minimal type for checking existing application status.
#[derive(Debug, Clone, Deserialize)]
struct Application {
    #[allow(dead_code)]
    id: String,
    status: String,
}

// Local request type (no shared equivalent for application submission).
#[derive(Serialize)]
struct ApplyBody {
    preferred_games: Vec<String>,
    preferred_roles: Vec<String>,
    message: Option<String>,
}

const APPLY_CSS: &str = r#"
    .apply-page { min-height: 100vh; padding: 2rem; max-width: 600px; margin: 0 auto; }
    .apply-title { font-family: var(--font-head); font-size: 2.5rem; color: var(--text); letter-spacing: 3px; text-align: center; margin-bottom: 2rem; }
    .apply-card-title { font-family: var(--font-head); font-weight: 700; font-size: 1.3rem; color: var(--text); margin: 0 0 0.5rem; }
    .apply-card-desc { color: var(--text-2); font-size: 0.9rem; line-height: 1.6; }
    .apply-auth-buttons { margin-top: 1.5rem; display: flex; gap: 0.75rem; flex-wrap: wrap; }
    .apply-status-row { margin: 1rem 0; }
    .apply-field { margin-top: 1.5rem; }
    .apply-label { font-family: var(--font-head); font-weight: 600; font-size: 0.85rem; color: var(--text); text-transform: uppercase; letter-spacing: 0.04em; display: block; margin-bottom: 0.5rem; }
    .apply-game-grid { display: flex; gap: 0.5rem; flex-wrap: wrap; }
    .apply-game-btn { padding: 0.4rem 1rem; border-radius: 6px; border: 1px solid var(--border); background: var(--surface); color: var(--text-2); font-size: 0.85rem; cursor: pointer; transition: all 0.15s; }
    .apply-game-btn:hover { border-color: var(--accent-soft); color: var(--text); }
    .apply-game-btn.selected { background: var(--accent); color: var(--accent-fg); border-color: var(--accent); }
    .apply-actions { margin-top: 1.5rem; }
    .apply-loading { color: var(--text-3); text-align: center; padding: 2rem; }
"#;

#[component]
pub fn Apply() -> Element {
    let auth = use_auth();

    let settings = use_api::<SiteSettings>("/api/settings");
    let games = use_api::<Vec<Game>>("/api/games");
    let mut my_app = use_api::<Option<Application>>("/api/applications/mine");

    let mut selected_games = use_signal(Vec::<String>::new);
    let mut message = use_signal(String::new);
    let mut submitting = use_signal(|| false);

    rsx! {
        style { {APPLY_CSS} }
        div { class: "apply-page",
            h1 { class: "apply-title", "Join The Scuffed Crew" }

            {
                let loading = auth().loading;
                let s = settings.data.read().as_ref().and_then(|s| s.as_ref()).cloned();

                if loading || s.is_none() {
                    rsx! { p { class: "apply-loading", "Loading..." } }
                } else {
                    let s = s.unwrap();

                    if !s.recruitment_open {
                        rsx! {
                            Card {
                                h2 { class: "apply-card-title", "Recruitment Closed" }
                                p { class: "apply-card-desc", "{s.recruitment_message}" }
                            }
                        }
                    } else if !auth().is_logged_in() {
                        rsx! {
                            Card {
                                h2 { class: "apply-card-title", "Log In to Apply" }
                                p { class: "apply-card-desc", "You need to sign in before submitting an application." }
                                div { class: "apply-auth-buttons",
                                    Link { to: Route::Login {}, class: "ui-btn ui-btn--primary ui-btn--md", "Sign in" }
                                    a {
                                        href: "/api/dev/login",
                                        class: "ui-btn ui-btn--md",
                                        style: "background: var(--surface); border: 1px solid var(--border); color: var(--text);",
                                        "Dev login"
                                    }
                                }
                            }
                        }
                    } else if let Some(app) = my_app.data.read().as_ref().and_then(|a| a.as_ref()).and_then(|a| a.as_ref()) {
                        let status_tone = match app.status.as_str() {
                            "pending" => PillTone::Warn,
                            "trial" => PillTone::Accent,
                            "accepted" => PillTone::Ok,
                            "rejected" => PillTone::Danger,
                            _ => PillTone::Neutral,
                        };
                        let status_label = match app.status.as_str() {
                            "pending" => "Pending Review",
                            "trial" => "Trial Period",
                            "accepted" => "Accepted",
                            "rejected" => "Rejected",
                            "withdrawn" => "Withdrawn",
                            _ => &app.status,
                        };
                        let desc = match app.status.as_str() {
                            "pending" => "Your application is being reviewed. We'll get back to you soon.",
                            "trial" => "You're in your trial period. Show up, have fun, and be yourself.",
                            "accepted" => "Welcome aboard! You're a member of The Scuffed Crew.",
                            "rejected" => "Unfortunately your application was not accepted at this time.",
                            _ => "",
                        };

                        rsx! {
                            Card {
                                h2 { class: "apply-card-title", "Application Status" }
                                div { class: "apply-status-row",
                                    Pill { tone: status_tone, "{status_label}" }
                                }
                                p { class: "apply-card-desc", "{desc}" }
                            }
                        }
                    } else {
                        let game_list = games.data.read().as_ref().and_then(|g| g.as_ref()).cloned().unwrap_or_default();

                        rsx! {
                            Card {
                                h2 { class: "apply-card-title", "Apply" }
                                p { class: "apply-card-desc", "Tell us which games you play and a bit about yourself." }

                                div { class: "apply-field",
                                    label { class: "apply-label", "Games" }
                                    div { class: "apply-game-grid",
                                        for g in game_list.iter() {
                                            {
                                                let gid = g.id.clone();
                                                let gid2 = g.id.clone();
                                                let is_selected = selected_games().contains(&gid);
                                                let btn_class = if is_selected { "apply-game-btn selected" } else { "apply-game-btn" };
                                                rsx! {
                                                    button {
                                                        class: "{btn_class}",
                                                        onclick: move |_| {
                                                            let gid = gid2.clone();
                                                            selected_games.write().retain(|x| x != &gid);
                                                            if !is_selected {
                                                                selected_games.write().push(gid);
                                                            }
                                                        },
                                                        "{g.name}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                div { class: "apply-field",
                                    label { class: "apply-label", "Message (optional)" }
                                    Textarea {
                                        value: message(),
                                        placeholder: "Tell us about yourself, your experience, what you're looking for...",
                                        oninput: move |e: FormEvent| message.set(e.value()),
                                    }
                                }

                                div { class: "apply-actions",
                                    Button {
                                        variant: BtnVariant::Primary,
                                        disabled: submitting(),
                                        onclick: move |_| {
                                            let games = selected_games();
                                            let msg = message();
                                            if games.is_empty() {
                                                let mut toast = use_toast();
                                                toast.show(Toast::error("Select at least one game"));
                                                return;
                                            }
                                            submitting.set(true);
                                            spawn(async move {
                                                let body = ApplyBody {
                                                    preferred_games: games,
                                                    preferred_roles: vec![],
                                                    message: if msg.trim().is_empty() { None } else { Some(msg) },
                                                };
                                                match ApiClient::web().post_json_empty("/api/applications", &body).await {
                                                    Ok(_) => {
                                                        let mut toast = use_toast();
                                                        toast.show(Toast::success("Application submitted!"));
                                                        my_app.refresh += 1;
                                                    }
                                                    Err(e) => {
                                                        let mut toast = use_toast();
                                                        toast.show(Toast::error(format!("Failed: {e}")));
                                                    }
                                                }
                                                submitting.set(false);
                                            });
                                        },
                                        if submitting() { "Submitting..." } else { "Submit Application" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
