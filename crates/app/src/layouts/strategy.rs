use dioxus::prelude::*;

use crate::routes::Route;

const STRATEGY_CSS: &str = r#"
    .strategy-nav {
        display: flex;
        align-items: center;
        gap: 1.5rem;
        padding: 0 2rem;
        height: 50px;
        background: var(--bg-surface);
        border-bottom: 1px solid var(--border);
    }
    .strategy-nav .brand {
        font-family: var(--font-display-hero);
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
        color: var(--text-secondary);
        font-size: 0.85rem;
        border-radius: 5px;
        transition: color 0.15s, background 0.15s;
    }
    .strategy-nav .links a:hover {
        color: var(--text-bright);
        background: var(--bg-card);
    }
    .strategy-nav .back-link {
        margin-left: auto;
        color: var(--text-muted);
        font-size: 0.8rem;
        transition: color 0.15s;
    }
    .strategy-nav .back-link:hover {
        color: var(--text-secondary);
    }
"#;

#[component]
pub fn StrategyLayout() -> Element {
    rsx! {
        div { "data-theme": "strategy",
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
