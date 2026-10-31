use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::ui::{LoadError, SkeletonRows};
use crate::routes::Route;
use crate::util::encode_query;
use scuffed_api_client::ApiClient;

// --- Types ---

#[derive(Debug, Clone, Deserialize)]
struct StrategySummary {
    id: String,
    name: String,
    map_id: String,
    game_mode: String,
    owner_name: String,
    #[allow(dead_code)]
    visibility: String,
    element_count: usize,
    updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ListResponse {
    /// Server serializes this as `data` (`StrategyListResponse`).
    #[serde(alias = "strategies")]
    data: Vec<StrategySummary>,
    #[allow(dead_code)]
    total: u64,
}

// --- Constants ---

const GAME_MODES: &[&str] = &[
    "All",
    "Escort",
    "Hybrid",
    "Control",
    "Push",
    "Flashpoint",
    "Clash",
];

const PAGE_CSS: &str = r#"
    .strategy-browse {
        padding: 2rem;
        max-width: 1200px;
        margin: 0 auto;
    }
    .strategy-browse-header {
        display: flex;
        justify-content: space-between;
        align-items: center;
        flex-wrap: wrap;
        gap: 1rem;
        margin-bottom: 1.5rem;
    }
    .strategy-browse-title {
        font-family: var(--font-head);
        font-size: 2rem;
        color: var(--text);
        text-transform: uppercase;
        letter-spacing: 2px;
    }
    .strategy-browse .btn-create {
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
    .strategy-browse .btn-create:hover {
        filter: brightness(1.15);
        box-shadow: 0 0 15px var(--accent-soft);
    }

    /* Search */
    .strategy-search {
        margin-bottom: 1rem;
    }
    .strategy-search-input {
        width: 100%;
        max-width: 400px;
        padding: 0.6rem 1rem;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--surface);
        color: var(--text);
        font-size: 0.9rem;
        font-family: inherit;
        transition: border-color 0.2s;
    }
    .strategy-search-input::placeholder {
        color: var(--text-3);
    }
    .strategy-search-input:focus {
        border-color: var(--accent);
    }

    /* Filter chips */
    .strategy-filters {
        display: flex;
        flex-wrap: wrap;
        gap: 0.4rem;
        margin-bottom: 1.5rem;
    }
    .strategy-browse .filter-chip {
        padding: 0.3rem 0.75rem;
        border-radius: 999px;
        border: 1px solid var(--border);
        background: var(--surface);
        color: var(--text-2);
        font-size: 0.78rem;
        font-weight: 600;
        cursor: pointer;
        transition: all 0.15s;
        text-transform: uppercase;
        letter-spacing: 0.03em;
    }
    .strategy-browse .filter-chip:hover {
        border-color: var(--accent-soft);
        color: var(--text);
    }
    .strategy-browse .filter-chip.active {
        background: var(--accent);
        border-color: var(--accent);
        color: white;
    }

    /* Card grid */
    .strategy-grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(min(280px, 100%), 1fr));
        gap: 1.25rem;
    }
    .strategy-card {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.25rem;
        text-decoration: none;
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
        transition: border-color 0.2s, transform 0.2s;
    }
    .strategy-card:hover {
        border-color: var(--accent-soft);
        transform: translateY(-2px);
    }
    .strategy-card-name {
        font-family: var(--font-head);
        font-weight: 700;
        font-size: 1.1rem;
        color: var(--text);
    }
    .strategy-card-meta {
        display: flex;
        gap: 0.75rem;
        flex-wrap: wrap;
        font-size: 0.75rem;
        color: var(--text-3);
    }
    .strategy-card-meta span {
        display: inline-flex;
        align-items: center;
        gap: 0.25rem;
    }
    .strategy-card-mode {
        display: inline-block;
        font-size: 0.65rem;
        padding: 0.15rem 0.5rem;
        border-radius: 999px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
        background: var(--accent-soft);
        color: var(--accent);
        width: fit-content;
    }
    .strategy-card-owner {
        color: var(--text-2);
        font-size: 0.8rem;
    }
    .strategy-card-footer {
        display: flex;
        justify-content: space-between;
        align-items: center;
        margin-top: auto;
        padding-top: 0.5rem;
        border-top: 1px solid var(--border);
        font-size: 0.7rem;
        color: var(--text-3);
    }

    /* States */
    .strategy-loading, .strategy-empty {
        color: var(--text-3);
        text-align: center;
        padding: 4rem 1rem;
        font-size: 0.95rem;
    }
    .strategy-empty-cta {
        margin-top: 1rem;
    }
"#;

// --- Component ---

/// `GET /api/strategy/strategies` with the search box and mode filter.
/// Both values are percent-encoded, so `&`, `#`, `+` and spaces in a search
/// stay part of it instead of splitting or cutting the query (APP10, FC-9).
fn strategy_list_path(search: &str, mode: &str) -> String {
    let mut path = String::from("/api/strategy/strategies");
    let mut params: Vec<String> = Vec::new();
    if !search.is_empty() {
        params.push(format!("search={}", encode_query(search)));
    }
    if mode != "All" {
        params.push(format!("game_mode={}", encode_query(mode)));
    }
    if !params.is_empty() {
        path.push('?');
        path.push_str(&params.join("&"));
    }
    path
}

#[component]
pub fn StrategyBrowse() -> Element {
    // Saving needs an org membership; non-members would draw and then get 403.
    let can_create = try_use_context::<Signal<crate::state::auth::AuthState>>()
        .is_some_and(|auth| auth.read().is_org_member());
    let mut search_input = use_signal(String::new);
    let mut debounced_search = use_signal(String::new);
    let mut selected_mode = use_signal(|| "All".to_string());

    // Debounce: sync search_input → debounced_search after 300ms
    let _debounce = use_resource(move || {
        let current = search_input();
        async move {
            #[cfg(feature = "web")]
            {
                gloo_timers::future::TimeoutFuture::new(300).await;
            }
            #[cfg(not(feature = "web"))]
            {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
            debounced_search.set(current);
        }
    });

    // Fetch strategies whenever debounced_search or selected_mode changes.
    // Result is Result so we can surface errors instead of eternal Loading.
    let mut strategies = use_resource(move || {
        let search = debounced_search();
        let mode = selected_mode();
        async move {
            let path = strategy_list_path(&search, &mode);
            ApiClient::web()
                .fetch::<ListResponse>(&path)
                .await
                .map_err(|e| e.to_string())
        }
    });

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-strategy-browse-page-css", css: PAGE_CSS }

        div { class: "strategy-browse",
            // Header
            div { class: "strategy-browse-header",
                h1 { class: "strategy-browse-title", "Browse Strategies" }
                if can_create {
                    Link { to: Route::StrategyEditorNew {}, class: "btn-create",
                        "+ Create New"
                    }
                }
            }

            // Search
            div { class: "strategy-search",
                input {
                    id: "strategy-search",
                    name: "strategy-search",
                    class: "strategy-search-input",
                    r#type: "text",
                    placeholder: "Search strategies...",
                    aria_label: "Search strategies",
                    value: "{search_input}",
                    oninput: move |e| search_input.set(e.value()),
                }
            }

            // Filter chips
            div { class: "strategy-filters",
                for mode in GAME_MODES.iter() {
                    {
                        let mode_str = mode.to_string();
                        let is_active = selected_mode() == *mode;
                        let chip_class = if is_active { "filter-chip active" } else { "filter-chip" };
                        rsx! {
                            button {
                                key: "{mode}",
                                class: "{chip_class}",
                                onclick: move |_| selected_mode.set(mode_str.clone()),
                                "{mode}"
                            }
                        }
                    }
                }
            }

            // Results
            {
                let data = strategies.read();
                match data.as_ref() {
                    None => rsx! {
                        SkeletonRows { status: "Loading strategies...".to_string() }
                    },
                    Some(Err(_)) => rsx! {
                        LoadError {
                            message: "Couldn't load strategies. Check your connection and try again."
                                .to_string(),
                            on_retry: move |_| strategies.restart(),
                        }
                    },
                    Some(Ok(resp)) if resp.data.is_empty() => rsx! {
                        div { class: "strategy-empty",
                            if !debounced_search().is_empty() || selected_mode() != "All" {
                                p { "No strategies match these filters." }
                                p { style: "margin-top:0.5rem; font-size:0.85rem;",
                                    "Try clearing search or switching the mode filter."
                                }
                            } else {
                                p { "No public strategies yet." }
                                p { style: "margin-top:0.5rem; font-size:0.85rem;",
                                    "Members can draft plans in Strategy → My Strategies and publish when ready."
                                }
                            }
                        }
                    },
                    Some(Ok(resp)) => rsx! {
                        div { class: "strategy-grid",
                            for strat in resp.data.iter() {
                                {render_strategy_card(strat)}
                            }
                        }
                    },
                }
            }
        }
    }
}

fn render_strategy_card(s: &StrategySummary) -> Element {
    let updated_short: String = s.updated_at.chars().take(10).collect();

    rsx! {
        Link {
            to: Route::StrategyEditor { id: s.id.clone() },
            class: "strategy-card",
            key: "{s.id}",

            div { class: "strategy-card-name", "{s.name}" }
            span { class: "strategy-card-mode", "{s.game_mode}" }
            div { class: "strategy-card-meta",
                span { "Map: {s.map_id}" }
            }
            div { class: "strategy-card-owner", "by {s.owner_name}" }
            div { class: "strategy-card-footer",
                span { "{s.element_count} elements" }
                span { "{updated_short}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_filters_is_the_bare_list() {
        assert_eq!(strategy_list_path("", "All"), "/api/strategy/strategies");
    }

    #[test]
    fn search_is_url_encoded() {
        assert_eq!(
            strategy_list_path("dive & poke", "All"),
            "/api/strategy/strategies?search=dive%20%26%20poke"
        );
        assert_eq!(
            strategy_list_path("C# rush", "All"),
            "/api/strategy/strategies?search=C%23%20rush"
        );
        assert_eq!(
            strategy_list_path("2+2", "All"),
            "/api/strategy/strategies?search=2%2B2"
        );
        assert_eq!(
            strategy_list_path("Lúcio speed", "All"),
            "/api/strategy/strategies?search=L%C3%BAcio%20speed"
        );
    }

    #[test]
    fn a_search_cannot_add_or_cut_parameters() {
        let path = strategy_list_path("a&game_mode=Push#x", "Control");
        assert_eq!(
            path,
            "/api/strategy/strategies?search=a%26game_mode%3DPush%23x&game_mode=Control"
        );
        assert_eq!(path.matches('&').count(), 1);
        assert!(!path.contains('#'));
    }
}
