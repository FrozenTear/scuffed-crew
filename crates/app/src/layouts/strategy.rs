use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use scuffed_types::SiteSettings;

use crate::routes::Route;

const STRATEGY_CSS: &str = r#"
    .strategy-nav {
        display: flex;
        align-items: center;
        gap: 1.5rem;
        padding: 0 2rem;
        height: 50px;
        background: var(--surface);
        border-bottom: 1px solid var(--border);
    }
    .strategy-nav .brand {
        font-family: var(--font-head);
        font-size: 0.95rem;
        color: var(--accent);
        text-transform: uppercase;
        letter-spacing: 0.06em;
    }
    .strategy-nav .links {
        display: flex;
        gap: 0.25rem;
    }
    .strategy-nav .links a {
        padding: 0.35rem 0.7rem;
        color: var(--text-2);
        font-size: 0.85rem;
        border-radius: 5px;
        transition: color 0.15s, background 0.15s;
    }
    .strategy-nav .links a:hover {
        color: var(--text);
        background: var(--surface-2);
    }
    .strategy-nav .back-link {
        margin-left: auto;
        color: var(--text-3);
        font-size: 0.8rem;
        transition: color 0.15s;
    }
    .strategy-nav .back-link:hover {
        color: var(--text-2);
    }
    .strategy-off {
        min-height: calc(100vh - 50px);
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        text-align: center;
        padding: 4rem 1.5rem;
        color: var(--text);
    }
    .strategy-off h1 {
        font-family: var(--font-head);
        font-size: 1.25rem;
        margin: 0 0 0.5rem;
    }
    .strategy-off p {
        color: var(--text-2);
        font-size: 0.95rem;
        max-width: 28rem;
        margin: 0 0 1.25rem;
        line-height: 1.5;
    }
    .strategy-off-links {
        display: flex;
        flex-wrap: wrap;
        gap: 0.75rem;
        justify-content: center;
    }
    .strategy-off a {
        display: inline-flex;
        align-items: center;
        padding: 0.55rem 1.1rem;
        background: var(--accent);
        color: var(--accent-fg);
        font-family: var(--font-mono);
        font-size: 0.72rem;
        letter-spacing: 0.1em;
        text-transform: uppercase;
        text-decoration: none;
        border-radius: var(--radius-md, 8px);
    }
    .strategy-off a.ghost {
        background: transparent;
        color: var(--text-2);
        border: 1px solid var(--border);
    }
"#;

/// Site-side gate. Reads `strategies_enabled` from GET /api/settings (default ON).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StrategyPathPolicy {
    Open,
    RedirectPatchNotes,
    Blocked,
}

fn strategies_enabled_or_default(settings: Option<&SiteSettings>) -> bool {
    settings.map(|s| s.strategies_enabled).unwrap_or(true)
}

fn strategy_path_policy(path: &str, strategies_enabled: bool) -> StrategyPathPolicy {
    if strategies_enabled {
        return StrategyPathPolicy::Open;
    }
    if path == "/strategy/patch-notes" {
        return StrategyPathPolicy::RedirectPatchNotes;
    }
    if path == "/strategy" || path.starts_with("/strategy/") {
        return StrategyPathPolicy::Blocked;
    }
    StrategyPathPolicy::Open
}

#[component]
pub fn StrategyLayout() -> Element {
    let navigator = use_navigator();
    let route = use_route::<Route>();
    let site_settings = use_resource(|| async {
        ApiClient::web()
            .fetch::<SiteSettings>("/api/settings")
            .await
            .ok()
    });
    let enabled =
        strategies_enabled_or_default(site_settings.read().as_ref().and_then(|o| o.as_ref()));
    let surface = strategy_path_policy(&route.to_string(), enabled);

    use_effect(move || {
        if surface == StrategyPathPolicy::RedirectPatchNotes {
            navigator.replace(Route::PatchNotes {});
        }
    });

    if surface == StrategyPathPolicy::Blocked {
        return rsx! {
            div { "data-accent": "strategy",
                style { {STRATEGY_CSS} }
                div { class: "strategy-off",
                    h1 { "Strategies unavailable" }
                    p {
                        "This clan has turned Strategies off. Patch Notes are still on the main site."
                    }
                    div { class: "strategy-off-links",
                        Link { to: Route::PatchNotes {}, "View Patch Notes" }
                        Link { to: Route::Home {}, class: "ghost", "Back to home" }
                    }
                }
            }
        };
    }

    rsx! {
        div { "data-accent": "strategy",
            style { {STRATEGY_CSS} }
            nav { class: "strategy-nav",
                span { class: "brand", "Strategy Planner" }
                div { class: "links",
                    Link { to: Route::StrategyBrowse {}, "Browse" }
                    Link { to: Route::StrategyMy {}, "My Strategies" }
                    Link { to: Route::StrategyHeroes {}, "Heroes" }
                    Link { to: Route::StrategyMeta {}, "Meta" }
                    Link { to: Route::StrategyPatchNotes {}, "Patch Notes" }
                    Link { to: Route::StrategyEditorNew {}, "New Strategy" }
                }
                Link { to: Route::Home {}, class: "back-link", "← Back to Clan" }
            }
            main { style: "min-height: calc(100vh - 50px);",
                Outlet::<Route> {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_keeps_planner_and_patch_notes_open() {
        assert_eq!(
            strategy_path_policy("/strategy", true),
            StrategyPathPolicy::Open
        );
        assert_eq!(
            strategy_path_policy("/strategy/patch-notes", true),
            StrategyPathPolicy::Open
        );
    }

    #[test]
    fn disabled_blocks_planner_keeps_public_patch_notes() {
        assert_eq!(
            strategy_path_policy("/strategy", false),
            StrategyPathPolicy::Blocked
        );
        assert_eq!(
            strategy_path_policy("/strategy/my", false),
            StrategyPathPolicy::Blocked
        );
        assert_eq!(
            strategy_path_policy("/strategy/editor/x", false),
            StrategyPathPolicy::Blocked
        );
        assert_eq!(
            strategy_path_policy("/strategy/patch-notes", false),
            StrategyPathPolicy::RedirectPatchNotes
        );
        assert_eq!(
            strategy_path_policy("/patch-notes", false),
            StrategyPathPolicy::Open
        );
    }
}
