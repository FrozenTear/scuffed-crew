use dioxus::prelude::*;
use scuffed_types::SiteSettings;

use crate::routes::Route;
use crate::state::use_site_settings;

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

/// Site-side gate. Uses the shared settings slot, including a `#sc-settings` seed.
/// A missing payload fail-opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StrategyPathPolicy {
    Open,
    /// Flag is off and this URL is the strategy alias of public Patch Notes.
    RedirectPatchNotes,
    /// GET /api/settings is still in flight on that alias. Not the fail-open default.
    AwaitingSettings,
    Blocked,
}

/// `None` while the settings resource has not settled.
/// `Some` once it has: a missing payload fail-opens, and a missing
/// `strategies_enabled` field is already `true` via serde before this runs.
fn strategy_path_policy(path: &str, strategies_enabled: Option<bool>) -> StrategyPathPolicy {
    match strategies_enabled {
        None if path == "/strategy/patch-notes" => StrategyPathPolicy::AwaitingSettings,
        None | Some(true) => StrategyPathPolicy::Open,
        Some(false) if path == "/strategy/patch-notes" => StrategyPathPolicy::RedirectPatchNotes,
        Some(false) if path == "/strategy" || path.starts_with("/strategy/") => {
            StrategyPathPolicy::Blocked
        }
        Some(false) => StrategyPathPolicy::Open,
    }
}

/// `None` while settings are still unknown. A seed or a successful fetch is
/// `Ok`. A failed fetch with nothing painted fail-opens (`Some(true)`).
fn flag_from_resolved(resolved: Option<&Result<SiteSettings, String>>) -> Option<bool> {
    match resolved {
        None => None,
        Some(Ok(settings)) => Some(settings.strategies_enabled),
        Some(Err(_)) => Some(true),
    }
}

#[component]
pub fn StrategyLayout() -> Element {
    let navigator = use_navigator();
    let site_settings = use_site_settings();
    // The memo reads `resolved()` once per settings change. Dioxus 0.7 effects
    // re-run only when signals are read *inside* the effect, so the effect
    // reads `flag()` (Copy) and `router().current()`. `use_route()` is a hook
    // and stays outside.
    let flag = use_memo(move || flag_from_resolved(site_settings.resolved().as_ref()));
    use_effect(move || {
        let path = router().current::<Route>().to_string();
        let enabled = flag();
        if strategy_path_policy(&path, enabled) == StrategyPathPolicy::RedirectPatchNotes {
            navigator.replace(Route::PatchNotes {});
        }
    });

    let path = router().current::<Route>().to_string();
    let surface = strategy_path_policy(&path, flag());

    if matches!(
        surface,
        StrategyPathPolicy::RedirectPatchNotes | StrategyPathPolicy::AwaitingSettings
    ) {
        let gate = if surface == StrategyPathPolicy::RedirectPatchNotes {
            "redirect"
        } else {
            "pending"
        };
        return rsx! {
            div { "data-accent": "strategy", "data-strategy-gate": "{gate}" }
        };
    }

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

    fn strategies_enabled_or_default(settings: Option<&SiteSettings>) -> bool {
        settings.map(|s| s.strategies_enabled).unwrap_or(true)
    }

    /// Outer `None`: resource still pending (do not apply the default).
    /// Inner `None`: settled with no settings payload → default on.
    fn flag_from_loaded_settings(loaded: Option<Option<&SiteSettings>>) -> Option<bool> {
        loaded.map(|settings| strategies_enabled_or_default(settings))
    }

    #[test]
    fn enabled_keeps_planner_and_patch_notes_open() {
        assert_eq!(
            strategy_path_policy("/strategy", Some(true)),
            StrategyPathPolicy::Open
        );
        assert_eq!(
            strategy_path_policy("/strategy/patch-notes", Some(true)),
            StrategyPathPolicy::Open
        );
    }

    #[test]
    fn disabled_blocks_planner_keeps_public_patch_notes() {
        assert_eq!(
            strategy_path_policy("/strategy", Some(false)),
            StrategyPathPolicy::Blocked
        );
        assert_eq!(
            strategy_path_policy("/strategy/my", Some(false)),
            StrategyPathPolicy::Blocked
        );
        assert_eq!(
            strategy_path_policy("/strategy/editor/x", Some(false)),
            StrategyPathPolicy::Blocked
        );
        assert_eq!(
            strategy_path_policy("/strategy/patch-notes", Some(false)),
            StrategyPathPolicy::RedirectPatchNotes
        );
        assert_eq!(
            strategy_path_policy("/patch-notes", Some(false)),
            StrategyPathPolicy::Open
        );
    }

    #[test]
    fn pending_patch_notes_is_not_the_fail_open_default() {
        assert_eq!(
            strategy_path_policy("/strategy/patch-notes", None),
            StrategyPathPolicy::AwaitingSettings
        );
        assert_eq!(
            strategy_path_policy("/strategy/patch-notes", Some(true)),
            StrategyPathPolicy::Open
        );
        assert!(strategies_enabled_or_default(None));
        assert_eq!(flag_from_loaded_settings(None), None);
        assert_eq!(flag_from_loaded_settings(Some(None)), Some(true));
        assert_eq!(flag_from_resolved(None), None);
        assert_eq!(flag_from_resolved(Some(&Err("offline".into()))), Some(true));
    }

    #[test]
    fn resolved_seed_uses_strategies_enabled() {
        let off = settings_with_flag(false);
        let on = settings_with_flag(true);
        assert_eq!(flag_from_resolved(Some(&Ok(off))), Some(false));
        assert_eq!(flag_from_resolved(Some(&Ok(on))), Some(true));
        assert_eq!(
            strategy_path_policy(
                "/strategy/patch-notes",
                flag_from_resolved(Some(&Ok(settings_with_flag(false))))
            ),
            StrategyPathPolicy::RedirectPatchNotes
        );
    }

    fn settings_with_flag(enabled: bool) -> SiteSettings {
        let flag = if enabled { "true" } else { "false" };
        let raw = format!(
            r#"{{"id":"site","org_name":"Org","site_description":"d","recruitment_open":true,"recruitment_message":"m","min_age":16,"forum_backend":"local","extra_relay_urls":"","strategies_enabled":{flag},"updated_at":"2026-10-06T00:00:00Z"}}"#
        );
        serde_json::from_str(&raw).expect("settings fixture")
    }

    #[test]
    fn pending_other_strategy_paths_stay_open() {
        assert_eq!(
            strategy_path_policy("/strategy", None),
            StrategyPathPolicy::Open
        );
        assert_eq!(
            strategy_path_policy("/strategy/my", None),
            StrategyPathPolicy::Open
        );
    }

    /// The redirect bug was the effect closing over a copied policy and never
    /// reading the settings resource or the route. Policy tests stay green either way.
    #[test]
    fn redirect_effect_subscribes_inside_the_effect() {
        let src = include_str!("strategy.rs");
        let memo_at = src.find("use_memo(move ||").expect("flag memo");
        let memo = &src[memo_at..memo_at + 120];
        assert!(
            memo.contains("resolved()") && memo.contains("flag_from_resolved"),
            "memo must read the shared settings slot, including an embedded seed"
        );
        let start = src.find("use_effect(move || {").expect("redirect effect");
        let body = &src[start..];
        let end = body.find("});").expect("effect end");
        let effect = &body[..end];
        assert!(
            effect.contains("flag()"),
            "effect must read the memo so it re-runs when settings settle"
        );
        assert!(
            !effect.contains("resolved()"),
            "resolved() belongs in the memo so StrategyLayout does not clone settings twice"
        );
        assert!(
            effect.contains("router().current"),
            "effect must read the current route inside so it re-runs on navigation"
        );
        assert!(
            !effect.contains("surface =="),
            "copied surface is not a signal; the effect would not re-run"
        );
    }
}
