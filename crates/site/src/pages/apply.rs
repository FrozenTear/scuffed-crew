use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};

use scuffed_auth::client::api::{fetch_json, post_json};
use scuffed_ui::components::button::{Button, ButtonVariant};
use scuffed_ui::components::toast::{use_toast, Toast};

use crate::app::use_site_auth;
use crate::components::Nav;
use crate::sections::Footer;

#[derive(Debug, Clone, Deserialize)]
struct SiteSettings {
    recruitment_open: bool,
    recruitment_message: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Game {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Application {
    #[allow(dead_code)]
    id: String,
    status: String,
}

#[derive(Serialize)]
struct ApplyBody {
    preferred_games: Vec<String>,
    preferred_roles: Vec<String>,
    message: Option<String>,
}

#[component]
pub fn ApplyPage() -> impl IntoView {
    let auth = use_site_auth();
    let toast = use_toast();

    let settings = LocalResource::new(|| async {
        fetch_json::<SiteSettings>("/api/settings").await.ok()
    });

    let games = LocalResource::new(|| async {
        fetch_json::<Vec<Game>>("/api/games").await.ok()
    });

    let app_refresh = RwSignal::new(0u32);
    let my_app = LocalResource::new(move || {
        app_refresh.get();
        async move {
            // This endpoint returns 401 if not logged in, which is fine
            fetch_json::<Option<Application>>("/api/applications/mine").await.ok().flatten()
        }
    });

    // Form state
    let selected_games = RwSignal::new(Vec::<String>::new());
    let message = RwSignal::new(String::new());
    let submitting = RwSignal::new(false);

    let toggle_game = move |game_id: String| {
        selected_games.update(|list| {
            if let Some(pos) = list.iter().position(|g| g == &game_id) {
                list.remove(pos);
            } else {
                list.push(game_id);
            }
        });
    };

    let do_submit = move || {
        let games = selected_games.get();
        let msg = message.get();

        if games.is_empty() {
            toast.show(Toast::error("Select at least one game"));
            return;
        }

        let body = ApplyBody {
            preferred_games: games,
            preferred_roles: vec![],
            message: if msg.trim().is_empty() { None } else { Some(msg) },
        };

        submitting.set(true);
        spawn_local(async move {
            match post_json::<_, serde_json::Value>("/api/applications", &body).await {
                Ok(_) => {
                    toast.show(Toast::success("Application submitted!"));
                    app_refresh.update(|n| *n += 1);
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
            submitting.set(false);
        });
    };

    view! {
        <Nav/>
        <main class="apply-page">
            <h1 class="apply-title">"Join The Scuffed Crew"</h1>

            {move || {
                let loading = auth.loading.get();
                let s = settings.get().flatten();

                if loading || s.is_none() {
                    return view! { <p class="apply-loading">"Loading..."</p> }.into_any();
                }

                let s = s.unwrap();

                // State 1: Recruitment closed
                if !s.recruitment_open {
                    return view! {
                        <div class="apply-card">
                            <h2 class="apply-card-title">"Recruitment Closed"</h2>
                            <p class="apply-card-desc">{s.recruitment_message}</p>
                        </div>
                    }.into_any();
                }

                // State 2: Not logged in
                if !auth.is_logged_in() {
                    return view! {
                        <div class="apply-card">
                            <h2 class="apply-card-title">"Log In to Apply"</h2>
                            <p class="apply-card-desc">"You need to sign in before submitting an application."</p>
                            <div class="apply-auth-buttons">
                                <a href="/api/auth/discord/login" class="btn btn-primary">"Sign in with Discord"</a>
                            </div>
                        </div>
                    }.into_any();
                }

                // State 3: Already applied
                let existing = my_app.get().flatten();
                if let Some(app) = existing {
                    let status_class = format!("apply-status-pill {}", app.status);
                    let status_label = match app.status.as_str() {
                        "pending" => "Pending Review",
                        "trial" => "Trial Period",
                        "accepted" => "Accepted",
                        "rejected" => "Rejected",
                        "withdrawn" => "Withdrawn",
                        _ => &app.status,
                    };
                    return view! {
                        <div class="apply-card">
                            <h2 class="apply-card-title">"Application Status"</h2>
                            <div class="apply-status-row">
                                <span class=status_class>{status_label}</span>
                            </div>
                            <p class="apply-card-desc">
                                {match app.status.as_str() {
                                    "pending" => "Your application is being reviewed. We'll get back to you soon.",
                                    "trial" => "You're in your trial period. Show up, have fun, and be yourself.",
                                    "accepted" => "Welcome aboard! You're a member of The Scuffed Crew.",
                                    "rejected" => "Unfortunately your application was not accepted at this time.",
                                    _ => "",
                                }}
                            </p>
                        </div>
                    }.into_any();
                }

                // State 4: Ready to apply
                let game_list = games.get().flatten().unwrap_or_default();
                view! {
                    <div class="apply-card">
                        <h2 class="apply-card-title">"Apply"</h2>
                        <p class="apply-card-desc">"Tell us which games you play and a bit about yourself."</p>

                        <div class="apply-field">
                            <label class="apply-label">"Games"</label>
                            <div class="apply-game-grid">
                                {game_list.into_iter().map(|g| {
                                    let gid = g.id.clone();
                                    let gid2 = g.id.clone();
                                    view! {
                                        <button
                                            class=move || {
                                                let sel = selected_games.get();
                                                if sel.contains(&gid) { "apply-game-btn selected" } else { "apply-game-btn" }
                                            }
                                            on:click=move |_| toggle_game(gid2.clone())
                                        >
                                            {g.name}
                                        </button>
                                    }
                                }).collect_view()}
                            </div>
                        </div>

                        <div class="apply-field">
                            <label class="apply-label">"Message (optional)"</label>
                            <textarea
                                class="apply-textarea"
                                rows=4
                                placeholder="Tell us about yourself, your experience, what you're looking for..."
                                prop:value=move || message.get()
                                on:input=move |ev| message.set(event_target_value(&ev))
                            />
                        </div>

                        <div class="apply-actions">
                            <Button
                                variant=ButtonVariant::Primary
                                disabled=submitting.get()
                                on_click=Callback::new(move |_| do_submit())
                            >
                                {move || if submitting.get() { "Submitting..." } else { "Submit Application" }}
                            </Button>
                        </div>
                    </div>
                }.into_any()
            }}
        </main>
        <Footer/>
    }
}
