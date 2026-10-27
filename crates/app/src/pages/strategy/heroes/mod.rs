//! Strategy Heroes page: every hero from API's shared hero list
//! (`GET /api/strategy/heroes`), grouped by role, with a detail panel.
//!
//! Split into small modules: `model` (decode and grouping), `grid` (left
//! panel), `detail` (right panel), `portrait` (image with fallback) and
//! `styles`.

mod detail;
mod grid;
mod model;
mod portrait;
mod styles;

use dioxus::prelude::*;

use scuffed_api_client::ApiClient;

use super::load_error::{heroes_load_error, is_retrying};
use crate::components::fetch_error_state;
use detail::render_hero_detail;
use grid::render_hero_grid;
use model::{Hero, ListResponse};
use styles::PAGE_CSS;

#[component]
pub fn StrategyHeroes() -> Element {
    let mut selected_hero: Signal<Option<Hero>> = use_signal(|| None);
    let mut search_filter: Signal<String> = use_signal(String::new);
    let refresh = use_signal(|| 0u32);

    let heroes = use_resource(move || {
        // A Retry bumps `refresh`; drop the old pick so it can't go stale.
        if refresh() > 0 {
            selected_hero.set(None);
        }
        async {
            ApiClient::web()
                .fetch::<ListResponse>("/api/strategy/heroes")
                .await
                .map(|resp| resp.data)
                .map_err(|e| e.to_string())
        }
    });

    let load_failed = matches!(heroes.read().as_ref(), Some(Err(_)));

    let left_panel = {
        let data = heroes.read();
        match data.as_ref() {
            None => rsx! { p { class: "heroes-loading", "Loading heroes..." } },
            Some(Err(err)) => {
                let shown = heroes_load_error(err);
                let retrying = is_retrying(*heroes.state().read());
                fetch_error_state(&shown.message, shown.retry.then_some(refresh), retrying)
            }
            Some(Ok(list)) if list.is_empty() => rsx! {
                p { class: "heroes-empty", "No heroes found." }
            },
            Some(Ok(list)) => render_hero_grid(list, &search_filter.read(), selected_hero),
        }
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-strategy-heroes-page-css", css: PAGE_CSS }

        div { class: "heroes-page",
            div { class: "heroes-left",
                if !load_failed {
                    input {
                        id: "strategy-hero-search",
                        name: "strategy-hero-search",
                        class: "heroes-search",
                        r#type: "search",
                        placeholder: "Search heroes...",
                        aria_label: "Search heroes",
                        value: "{search_filter}",
                        oninput: move |e| search_filter.set(e.value()),
                    }
                }
                {left_panel}
            }

            if !load_failed {
                div { class: "heroes-right",
                    if let Some(hero) = selected_hero() {
                        {render_hero_detail(&hero)}
                    } else {
                        div { class: "heroes-placeholder", "Select a hero to view details" }
                    }
                }
            }
        }
    }
}
