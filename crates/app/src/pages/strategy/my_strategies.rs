use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;
use crate::components::{ConfirmDialog, Toast, use_toast};
use crate::hooks::use_api;
use crate::routes::Route;
use crate::state::auth::use_auth;

// --- Types ---

#[derive(Debug, Clone, Deserialize)]
struct StrategySummary {
    id: String,
    name: String,
    map_id: String,
    game_mode: String,
    #[allow(dead_code)]
    owner_name: String,
    visibility: String,
    element_count: usize,
    updated_at: String,
}

// --- CSS ---

const PAGE_CSS: &str = r#"
    .my-strategies {
        padding: 2rem;
        max-width: 1200px;
        margin: 0 auto;
    }
    .my-strategies-header {
        display: flex;
        justify-content: space-between;
        align-items: center;
        flex-wrap: wrap;
        gap: 1rem;
        margin-bottom: 1.5rem;
    }
    .my-strategies-title {
        font-family: var(--font-display-hero);
        font-size: 2rem;
        color: var(--text-bright);
        text-transform: uppercase;
        letter-spacing: 2px;
    }
    .btn-create {
        display: inline-flex;
        align-items: center;
        gap: 0.4rem;
        padding: 0.55rem 1.3rem;
        border-radius: 6px;
        background: var(--accent);
        color: white;
        border: none;
        font-size: 0.85rem;
        font-weight: 600;
        cursor: pointer;
        text-transform: uppercase;
        letter-spacing: 0.03em;
        text-decoration: none;
        transition: all 0.2s;
    }
    .btn-create:hover {
        filter: brightness(1.15);
        box-shadow: 0 0 15px var(--accent-glow);
    }

    /* Card grid */
    .my-strategy-grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
        gap: 1.25rem;
    }
    .my-strategy-card {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.25rem;
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
        transition: border-color 0.2s;
    }
    .my-strategy-card:hover {
        border-color: var(--border-light);
    }
    .my-strategy-card-name {
        font-family: var(--font-display);
        font-weight: 700;
        font-size: 1.1rem;
        color: var(--text-bright);
    }
    .my-strategy-card-meta {
        display: flex;
        gap: 0.75rem;
        flex-wrap: wrap;
        font-size: 0.75rem;
        color: var(--text-muted);
    }
    .my-strategy-card-mode {
        display: inline-block;
        font-size: 0.65rem;
        padding: 0.15rem 0.5rem;
        border-radius: 999px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
        background: var(--accent-soft);
        color: var(--accent-bright);
        width: fit-content;
    }

    /* Visibility badges */
    .visibility-badge {
        display: inline-block;
        font-size: 0.6rem;
        padding: 0.12rem 0.45rem;
        border-radius: 999px;
        font-weight: 700;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .visibility-badge.public {
        background: #22c55e33;
        color: #4ade80;
    }
    .visibility-badge.unlisted {
        background: #fbbf2433;
        color: #fbbf24;
    }
    .visibility-badge.private {
        background: #6b728033;
        color: #9ca3af;
    }

    .my-strategy-card-footer {
        display: flex;
        justify-content: space-between;
        align-items: center;
        margin-top: auto;
        padding-top: 0.5rem;
        border-top: 1px solid var(--border);
        font-size: 0.7rem;
        color: var(--text-muted);
    }

    /* Card actions */
    .my-strategy-card-actions {
        display: flex;
        gap: 0.4rem;
        margin-top: 0.5rem;
    }
    .card-btn {
        padding: 0.3rem 0.65rem;
        border-radius: 5px;
        border: 1px solid var(--border);
        background: var(--bg-surface);
        color: var(--text-secondary);
        font-size: 0.75rem;
        font-weight: 600;
        cursor: pointer;
        transition: all 0.15s;
        text-decoration: none;
        text-transform: uppercase;
        letter-spacing: 0.02em;
    }
    .card-btn:hover {
        border-color: var(--accent-soft);
        color: var(--text-bright);
    }
    .card-btn.danger:hover {
        border-color: #f87171;
        color: #f87171;
    }

    /* States */
    .my-strategies-loading, .my-strategies-empty {
        color: var(--text-muted);
        text-align: center;
        padding: 4rem 1rem;
        font-size: 0.95rem;
    }
    .my-strategies-login {
        text-align: center;
        padding: 5rem 2rem;
    }
    .my-strategies-login h2 {
        font-family: var(--font-display);
        font-size: 1.4rem;
        color: var(--text-bright);
        margin-bottom: 0.75rem;
    }
    .my-strategies-login p {
        color: var(--text-secondary);
        margin-bottom: 1.5rem;
        font-size: 0.9rem;
    }
    .login-link {
        display: inline-block;
        padding: 0.5rem 1.5rem;
        border-radius: 6px;
        background: var(--accent);
        color: white;
        font-weight: 600;
        font-size: 0.85rem;
        text-decoration: none;
        transition: all 0.2s;
    }
    .login-link:hover {
        filter: brightness(1.15);
        box-shadow: 0 0 15px var(--accent-glow);
    }
"#;

// --- Component ---

#[component]
pub fn StrategyMy() -> Element {
    let auth = use_auth();
    let mut toast = use_toast();
    let mut strategies = use_api::<Vec<StrategySummary>>("/api/strategy/strategies/mine");

    // Delete confirmation state
    let mut delete_open = use_signal(|| false);
    let mut delete_target: Signal<Option<StrategySummary>> = use_signal(|| None);

    // Auth guard
    let auth_state = auth();
    if auth_state.loading {
        return rsx! {
            style { {PAGE_CSS} }
            div { class: "my-strategies",
                p { class: "my-strategies-loading", "Loading..." }
            }
        };
    }

    if !auth_state.is_logged_in() {
        return rsx! {
            style { {PAGE_CSS} }
            div { class: "my-strategies",
                div { class: "my-strategies-login",
                    h2 { "Log in to view your strategies" }
                    p { "You need to be signed in to manage your personal strategy library." }
                    a { href: "/api/auth/login", class: "login-link", "Log In" }
                }
            }
        };
    }

    // --- Delete handlers ---

    let mut open_delete = move |strat: StrategySummary| {
        delete_target.set(Some(strat));
        delete_open.set(true);
    };

    let on_delete_confirm = move |_| {
        if let Some(strat) = delete_target() {
            let id = strat.id.clone();
            spawn(async move {
                match ApiClient::web()
                    .delete(&format!("/api/strategy/strategies/{id}"))
                    .await
                {
                    Ok(_) => {
                        toast.show(Toast::success("Strategy deleted."));
                        delete_open.set(false);
                        delete_target.set(None);
                        strategies.refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error(format!("Delete failed: {e}"))),
                }
            });
        }
    };

    let on_delete_cancel = move |_| {
        delete_open.set(false);
        delete_target.set(None);
    };

    // --- Render ---

    rsx! {
        style { {PAGE_CSS} }
        style { {crate::styles::admin::CSS} }

        div { class: "my-strategies",
            // Header
            div { class: "my-strategies-header",
                h1 { class: "my-strategies-title", "My Strategies" }
                Link { to: Route::StrategyEditorNew {}, class: "btn-create",
                    "+ Create New"
                }
            }

            // List
            {
                let data = strategies.data.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! {
                        p { class: "my-strategies-loading", "Loading your strategies..." }
                    },
                    Some(list) if list.is_empty() => rsx! {
                        div { class: "my-strategies-empty",
                            p { "No strategies yet \u{2014} create your first!" }
                            div { style: "margin-top:1rem;",
                                Link { to: Route::StrategyEditorNew {}, class: "btn-create",
                                    "+ Create Strategy"
                                }
                            }
                        }
                    },
                    Some(list) => rsx! {
                        div { class: "my-strategy-grid",
                            for strat in list.iter() {
                                {
                                    let s = strat.clone();
                                    let s_del = strat.clone();
                                    let updated_short: String = strat.updated_at.chars().take(10).collect();
                                    let vis_class = format!("visibility-badge {}", strat.visibility);
                                    rsx! {
                                        div { class: "my-strategy-card", key: "{strat.id}",
                                            div { style: "display:flex; justify-content:space-between; align-items:flex-start; gap:0.5rem;",
                                                div { class: "my-strategy-card-name", "{s.name}" }
                                                span { class: "{vis_class}", "{s.visibility}" }
                                            }
                                            span { class: "my-strategy-card-mode", "{s.game_mode}" }
                                            div { class: "my-strategy-card-meta",
                                                span { "Map: {s.map_id}" }
                                            }
                                            div { class: "my-strategy-card-footer",
                                                span { "{s.element_count} elements" }
                                                span { "{updated_short}" }
                                            }
                                            div { class: "my-strategy-card-actions",
                                                Link {
                                                    to: Route::StrategyEditor { id: s.id.clone() },
                                                    class: "card-btn",
                                                    "Edit"
                                                }
                                                button {
                                                    class: "card-btn danger",
                                                    onclick: move |_| open_delete(s_del.clone()),
                                                    "Delete"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    },
                }
            }

            // Delete confirmation dialog
            ConfirmDialog {
                title: "Delete Strategy".to_string(),
                message: format!(
                    "Are you sure you want to delete \"{}\"? This cannot be undone.",
                    delete_target().map(|s| s.name).unwrap_or_default()
                ),
                open: delete_open(),
                danger: true,
                on_confirm: on_delete_confirm,
                on_cancel: on_delete_cancel,
            }
        }
    }
}
