use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::{Toast, use_toast};
use crate::hooks::use_api;
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
    .apply-title { font-family: 'Bebas Neue', sans-serif; font-size: 2.5rem; color: var(--text-bright); letter-spacing: 3px; text-align: center; margin-bottom: 2rem; }
    .apply-card { background: var(--bg-card); border: 1px solid var(--border); border-radius: 12px; padding: 2rem; }
    .apply-card-title { font-family: 'Rajdhani', sans-serif; font-weight: 700; font-size: 1.3rem; color: var(--text-bright); margin: 0 0 0.5rem; }
    .apply-card-desc { color: var(--text-secondary); font-size: 0.9rem; line-height: 1.6; }
    .apply-auth-buttons { margin-top: 1.5rem; display: flex; gap: 0.75rem; flex-wrap: wrap; }
    .apply-auth-buttons a { display: inline-flex; align-items: center; gap: 0.5rem; padding: 0.6rem 1.4rem; border-radius: 6px; font-size: 0.9rem; font-weight: 600; text-transform: uppercase; letter-spacing: 0.03em; text-decoration: none; transition: all 0.2s; background: var(--accent); color: white; }
    .apply-auth-buttons a:hover { filter: brightness(1.15); }
    .apply-status-row { margin: 1rem 0; }
    .apply-status-pill { display: inline-block; padding: 0.2rem 0.75rem; border-radius: 999px; font-size: 0.75rem; font-weight: 700; text-transform: uppercase; letter-spacing: 0.04em; }
    .apply-status-pill.pending { background: #f59e0b33; color: #fbbf24; }
    .apply-status-pill.trial { background: #3b82f633; color: #60a5fa; }
    .apply-status-pill.accepted { background: #10b98133; color: #34d399; }
    .apply-status-pill.rejected { background: #ef444433; color: #f87171; }
    .apply-field { margin-top: 1.5rem; }
    .apply-label { font-family: 'Rajdhani', sans-serif; font-weight: 600; font-size: 0.85rem; color: var(--text-bright); text-transform: uppercase; letter-spacing: 0.04em; display: block; margin-bottom: 0.5rem; }
    .apply-game-grid { display: flex; gap: 0.5rem; flex-wrap: wrap; }
    .apply-game-btn { padding: 0.4rem 1rem; border-radius: 6px; border: 1px solid var(--border); background: var(--bg-card); color: var(--text-secondary); font-size: 0.85rem; cursor: pointer; transition: all 0.15s; }
    .apply-game-btn:hover { border-color: var(--accent-soft); color: var(--text-bright); }
    .apply-game-btn.selected { background: var(--accent); color: white; border-color: var(--accent); }
    .apply-textarea { width: 100%; background: var(--bg-surface); border: 1px solid var(--border); border-radius: 8px; color: var(--text-bright); padding: 0.75rem; font-size: 0.9rem; font-family: inherit; resize: vertical; }
    .apply-textarea:focus { outline: none; border-color: var(--accent); }
    .apply-actions { margin-top: 1.5rem; }
    .apply-btn { display: inline-flex; align-items: center; gap: 0.5rem; padding: 0.6rem 1.4rem; border-radius: 6px; font-size: 0.9rem; font-weight: 600; text-transform: uppercase; letter-spacing: 0.03em; text-decoration: none; transition: all 0.2s; border: none; cursor: pointer; background: var(--accent); color: white; }
    .apply-btn:hover { filter: brightness(1.15); box-shadow: 0 0 20px var(--accent-glow); }
    .apply-btn:disabled { opacity: 0.5; cursor: not-allowed; }
    .apply-loading { color: var(--text-muted); text-align: center; padding: 2rem; }
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
                            div { class: "apply-card",
                                h2 { class: "apply-card-title", "Recruitment Closed" }
                                p { class: "apply-card-desc", "{s.recruitment_message}" }
                            }
                        }
                    } else if !auth().is_logged_in() {
                        rsx! {
                            div { class: "apply-card",
                                h2 { class: "apply-card-title", "Log In to Apply" }
                                p { class: "apply-card-desc", "You need to sign in before submitting an application." }
                                div { class: "apply-auth-buttons",
                                    a { href: "/api/auth/discord/login", "Sign in with Discord" }
                                }
                            }
                        }
                    } else if let Some(app) = my_app.data.read().as_ref().and_then(|a| a.as_ref()).and_then(|a| a.as_ref()) {
                        let status_class = format!("apply-status-pill {}", app.status);
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
                            div { class: "apply-card",
                                h2 { class: "apply-card-title", "Application Status" }
                                div { class: "apply-status-row",
                                    span { class: "{status_class}", "{status_label}" }
                                }
                                p { class: "apply-card-desc", "{desc}" }
                            }
                        }
                    } else {
                        let game_list = games.data.read().as_ref().and_then(|g| g.as_ref()).cloned().unwrap_or_default();

                        rsx! {
                            div { class: "apply-card",
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
                                    textarea {
                                        class: "apply-textarea",
                                        rows: 4,
                                        placeholder: "Tell us about yourself, your experience, what you're looking for...",
                                        value: "{message}",
                                        oninput: move |e| message.set(e.value()),
                                    }
                                }

                                div { class: "apply-actions",
                                    button {
                                        class: "apply-btn",
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
