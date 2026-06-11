use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;

// --- Types ---

#[derive(Debug, Clone, Deserialize)]
struct Patch {
    version: String,
    date: String,
    title: Option<String>,
    url: String,
    hero_updates: Vec<HeroUpdate>,
    sections: Vec<PatchSection>,
}

#[derive(Debug, Clone, Deserialize)]
struct HeroUpdate {
    hero_id: String,
    hero_name: String,
    change_type: String,
    changes: Vec<PatchChange>,
    dev_comment: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct PatchChange {
    ability: Option<String>,
    description: String,
    #[allow(dead_code)]
    change_type: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PatchSection {
    category: String,
    items: Vec<String>,
}

// --- Filter categories ---

const FILTER_OPTIONS: [&str; 6] = [
    "All",
    "Hero Balance",
    "Bug Fixes",
    "Maps",
    "Competitive",
    "General",
];

fn patch_matches_filter(patch: &Patch, filter: &str) -> bool {
    match filter {
        "All" => true,
        "Hero Balance" => !patch.hero_updates.is_empty(),
        "Bug Fixes" => {
            patch.hero_updates.iter().any(|h| h.change_type == "bugfix")
                || patch.sections.iter().any(|s| {
                    let cat = s.category.to_lowercase();
                    cat.contains("bug") || cat.contains("fix")
                })
        }
        "Maps" => patch.sections.iter().any(|s| {
            let cat = s.category.to_lowercase();
            cat.contains("map")
        }),
        "Competitive" => patch.sections.iter().any(|s| {
            let cat = s.category.to_lowercase();
            cat.contains("competitive") || cat.contains("ranked")
        }),
        "General" => patch.sections.iter().any(|s| {
            let cat = s.category.to_lowercase();
            cat.contains("general") || cat.contains("system") || cat.contains("ui")
        }),
        _ => true,
    }
}

fn patch_matches_search(patch: &Patch, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let q = query.to_lowercase();
    if patch.version.to_lowercase().contains(&q) {
        return true;
    }
    if let Some(title) = &patch.title
        && title.to_lowercase().contains(&q)
    {
        return true;
    }
    patch
        .hero_updates
        .iter()
        .any(|h| h.hero_name.to_lowercase().contains(&q))
}

// --- Change type helpers ---

fn change_type_color(ct: &str) -> &'static str {
    match ct {
        "buff" => "var(--ok)",
        "nerf" => "var(--danger)",
        "adjustment" => "var(--warn)",
        "bugfix" => "var(--text-3)",
        _ => "var(--text-3)",
    }
}

fn change_type_bg(ct: &str) -> &'static str {
    match ct {
        "buff" => "color-mix(in srgb, var(--ok) 12%, transparent)",
        "nerf" => "color-mix(in srgb, var(--danger) 12%, transparent)",
        "adjustment" => "color-mix(in srgb, var(--warn) 12%, transparent)",
        "bugfix" => "color-mix(in srgb, var(--text-3) 12%, transparent)",
        _ => "color-mix(in srgb, var(--text-3) 12%, transparent)",
    }
}

fn change_type_label(ct: &str) -> &'static str {
    match ct {
        "buff" => "Buff",
        "nerf" => "Nerf",
        "adjustment" => "Adjustment",
        "bugfix" => "Bug Fix",
        _ => "Other",
    }
}

fn section_tag_color(category: &str) -> &'static str {
    let cat = category.to_lowercase();
    if cat.contains("hero") || cat.contains("balance") {
        "var(--ok)"
    } else if cat.contains("bug") || cat.contains("fix") {
        "var(--text-3)"
    } else if cat.contains("map") {
        "var(--chart-5)"
    } else if cat.contains("competitive") || cat.contains("ranked") {
        "var(--accent)"
    } else {
        "var(--text-3)"
    }
}

// --- CSS ---

const PAGE_CSS: &str = r#"
    .patch-page {
        padding: 2rem;
        max-width: 900px;
        margin: 0 auto;
    }
    .patch-header {
        display: flex;
        align-items: center;
        gap: 1rem;
        margin-bottom: 1.5rem;
        flex-wrap: wrap;
    }
    .patch-page-title {
        font-family: var(--font-head);
        font-size: 2.2rem;
        color: var(--text);
        letter-spacing: 2px;
        text-transform: uppercase;
        margin: 0;
        flex-shrink: 0;
    }
    .patch-search {
        flex: 1;
        min-width: 200px;
        padding: 0.45rem 0.75rem;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--surface);
        color: var(--text);
        font-size: 0.85rem;
        font-family: var(--font-body);
        outline: none;
        transition: border-color 0.15s;
    }
    .patch-search::placeholder {
        color: var(--text-3);
    }
    .patch-search:focus {
        border-color: var(--accent);
    }
    .patch-filters {
        display: flex;
        gap: 0.5rem;
        margin-bottom: 1.5rem;
        flex-wrap: wrap;
    }
    .patch-chip {
        padding: 0.3rem 0.7rem;
        border-radius: 999px;
        font-size: 0.75rem;
        font-weight: 600;
        border: 1px solid var(--border);
        background: var(--surface);
        color: var(--text-2);
        cursor: pointer;
        transition: all 0.15s;
        text-transform: uppercase;
        letter-spacing: 0.03em;
    }
    .patch-chip:hover {
        border-color: var(--accent-soft);
        color: var(--text);
    }
    .patch-chip.active {
        border-color: var(--accent);
        background: var(--accent-soft);
        color: var(--accent);
    }
    .patch-timeline {
        display: flex;
        flex-direction: column;
        gap: 1rem;
    }
    .patch-card {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 8px;
        overflow: hidden;
        transition: border-color 0.2s;
    }
    .patch-card:hover {
        border-color: var(--accent-soft);
    }
    .patch-card-header {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        padding: 0.85rem 1.1rem;
        cursor: pointer;
        user-select: none;
        flex-wrap: wrap;
    }
    .patch-card-header:hover {
        background: var(--surface-2);
    }
    .patch-version-badge {
        font-family: var(--font-mono);
        font-size: 0.78rem;
        font-weight: 600;
        padding: 0.15rem 0.55rem;
        border-radius: 4px;
        background: var(--accent-soft);
        color: var(--accent);
        flex-shrink: 0;
    }
    .patch-card-title {
        font-family: var(--font-head);
        font-weight: 700;
        font-size: 0.95rem;
        color: var(--text);
        flex: 1;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .patch-card-date {
        font-size: 0.75rem;
        color: var(--text-3);
        flex-shrink: 0;
    }
    .patch-hero-count {
        font-size: 0.7rem;
        color: var(--text-2);
        background: var(--surface-2);
        padding: 0.1rem 0.45rem;
        border-radius: 999px;
        flex-shrink: 0;
    }
    .patch-tag-pills {
        display: flex;
        gap: 0.35rem;
        flex-wrap: wrap;
    }
    .patch-tag-pill {
        font-size: 0.6rem;
        padding: 0.08rem 0.4rem;
        border-radius: 999px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.03em;
    }
    .patch-expand-icon {
        font-size: 0.7rem;
        color: var(--text-3);
        flex-shrink: 0;
        transition: transform 0.2s;
    }
    .patch-expand-icon.open {
        transform: rotate(180deg);
    }
    .patch-card-body {
        padding: 0 1.1rem 1.1rem;
        border-top: 1px solid var(--border);
    }
    .patch-section-title {
        font-family: var(--font-head);
        font-weight: 700;
        font-size: 0.85rem;
        color: var(--text);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        margin: 1rem 0 0.6rem;
    }
    .patch-hero-cards {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
    }
    .patch-hero-card {
        background: var(--surface-2);
        border: 1px solid var(--border);
        border-radius: 6px;
        padding: 0.85rem 1rem;
    }
    .patch-hero-card-header {
        display: flex;
        align-items: center;
        gap: 0.6rem;
        margin-bottom: 0.5rem;
    }
    .patch-hero-name {
        font-family: var(--font-head);
        font-weight: 700;
        font-size: 0.9rem;
        color: var(--text);
    }
    .patch-change-badge {
        font-size: 0.6rem;
        padding: 0.1rem 0.45rem;
        border-radius: 999px;
        font-weight: 700;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .patch-dev-comment {
        font-size: 0.78rem;
        color: var(--text-2);
        font-style: italic;
        padding: 0.5rem 0.75rem;
        margin: 0.4rem 0 0.6rem;
        border-left: 2px solid var(--border);
        line-height: 1.5;
    }
    .patch-change-list {
        list-style: none;
        padding: 0;
        margin: 0;
        display: flex;
        flex-direction: column;
        gap: 0.3rem;
    }
    .patch-change-item {
        font-size: 0.8rem;
        color: var(--text);
        line-height: 1.5;
        padding-left: 0.75rem;
        position: relative;
    }
    .patch-change-item::before {
        content: "\2022";
        position: absolute;
        left: 0;
        color: var(--text-3);
    }
    .patch-change-ability {
        font-weight: 700;
        color: var(--text);
    }
    .patch-section-items {
        list-style: none;
        padding: 0;
        margin: 0;
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
    }
    .patch-section-item {
        font-size: 0.8rem;
        color: var(--text);
        line-height: 1.5;
        padding-left: 0.75rem;
        position: relative;
    }
    .patch-section-item::before {
        content: "\2022";
        position: absolute;
        left: 0;
        color: var(--text-3);
    }
    .patch-external-link {
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
        font-size: 0.75rem;
        color: var(--accent);
        text-decoration: none;
        margin-top: 0.75rem;
        transition: opacity 0.15s;
    }
    .patch-external-link:hover {
        opacity: 0.8;
        text-decoration: underline;
    }
    .patch-loading, .patch-empty {
        color: var(--text-3);
        text-align: center;
        padding: 3rem 0;
    }
"#;

// --- Component ---

#[component]
pub fn StrategyPatchNotes() -> Element {
    let patches_data = use_resource(|| async {
        ApiClient::web()
            .fetch::<Vec<Patch>>("/api/strategy/patch-notes")
            .await
            .ok()
    });

    let mut search_query = use_signal(String::new);
    let mut active_filter = use_signal(|| "All".to_string());
    let mut expanded: Signal<Vec<usize>> = use_signal(|| vec![0]);

    rsx! {
        style { {PAGE_CSS} }

        div { class: "patch-page",
            // Title bar with search
            div { class: "patch-header",
                h1 { class: "patch-page-title", "Patch Notes" }
                input {
                    class: "patch-search",
                    placeholder: "Search version, title, or hero...",
                    value: "{search_query}",
                    oninput: move |evt| search_query.set(evt.value()),
                }
            }

            // Filter chips
            div { class: "patch-filters",
                for label in FILTER_OPTIONS {
                    {render_filter_chip(label, &(active_filter)(), &mut active_filter)}
                }
            }

            {
                let data = patches_data.read();
                let data = data.as_ref().and_then(|d| d.as_ref());

                match data {
                    None => rsx! { p { class: "patch-loading", "Loading patch notes..." } },
                    Some(patches) if patches.is_empty() => rsx! {
                        p { class: "patch-empty", "No patch notes available." }
                    },
                    Some(patches) => {
                        let query = (search_query)();
                        let filter = (active_filter)();
                        let visible: Vec<(usize, &Patch)> = patches
                            .iter()
                            .enumerate()
                            .filter(|(_, p)| patch_matches_search(p, &query) && patch_matches_filter(p, &filter))
                            .collect();

                        if visible.is_empty() {
                            rsx! {
                                p { class: "patch-empty", "No patches match your search." }
                            }
                        } else {
                            let expanded_indices = (expanded)();
                            rsx! {
                                div { class: "patch-timeline",
                                    for (idx, patch) in visible.iter() {
                                        {render_patch_card(*idx, patch, expanded_indices.contains(idx), &mut expanded)}
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

// --- Render helpers ---

fn render_filter_chip(label: &str, current: &str, signal: &mut Signal<String>) -> Element {
    let is_active = label == current;
    let class = if is_active {
        "patch-chip active"
    } else {
        "patch-chip"
    };
    let label_owned = label.to_string();
    let mut sig = *signal;

    rsx! {
        button {
            class: "{class}",
            onclick: move |_| sig.set(label_owned.clone()),
            "{label}"
        }
    }
}

fn render_patch_card(
    idx: usize,
    patch: &Patch,
    is_expanded: bool,
    expanded_signal: &mut Signal<Vec<usize>>,
) -> Element {
    let title = patch
        .title
        .clone()
        .unwrap_or_else(|| format!("Patch {}", patch.version));
    let hero_count = patch.hero_updates.len();
    let expand_class = if is_expanded {
        "patch-expand-icon open"
    } else {
        "patch-expand-icon"
    };

    // Collect unique section categories for tag pills
    let mut categories: Vec<String> = Vec::new();
    if !patch.hero_updates.is_empty() {
        categories.push("Hero Balance".to_string());
    }
    for s in &patch.sections {
        if !categories.iter().any(|c| c == &s.category) {
            categories.push(s.category.clone());
        }
    }

    let version = patch.version.clone();
    let date = patch.date.clone();
    let url = patch.url.clone();
    let hero_updates = patch.hero_updates.clone();
    let sections = patch.sections.clone();

    let mut sig = *expanded_signal;

    rsx! {
        div { class: "patch-card",
            // Collapsed header — always visible
            div {
                class: "patch-card-header",
                onclick: move |_| {
                    let mut current = sig();
                    if let Some(pos) = current.iter().position(|&i| i == idx) {
                        current.remove(pos);
                    } else {
                        current.push(idx);
                    }
                    sig.set(current);
                },

                span { class: "patch-version-badge", "{version}" }
                span { class: "patch-card-title", "{title}" }
                span { class: "patch-card-date", "{date}" }
                if hero_count > 0 {
                    {
                        let suffix = if hero_count != 1 { "es" } else { "" };
                        let label = format!("{hero_count} hero{suffix}");
                        rsx! { span { class: "patch-hero-count", "{label}" } }
                    }
                }
                div { class: "patch-tag-pills",
                    for cat in categories.iter() {
                        {render_tag_pill(cat)}
                    }
                }
                span { class: "{expand_class}", "\u{25bc}" }
            }

            // Expanded body
            if is_expanded {
                div { class: "patch-card-body",
                    // Hero balance section
                    if !hero_updates.is_empty() {
                        h3 { class: "patch-section-title", "Hero Balance" }
                        div { class: "patch-hero-cards",
                            for hu in hero_updates.iter() {
                                {render_hero_update(hu)}
                            }
                        }
                    }

                    // Other sections
                    for section in sections.iter() {
                        {render_section(section)}
                    }

                    // External link
                    a {
                        class: "patch-external-link",
                        href: "{url}",
                        target: "_blank",
                        rel: "noopener noreferrer",
                        "View official patch notes \u{2192}"
                    }
                }
            }
        }
    }
}

fn render_tag_pill(category: &str) -> Element {
    let color = section_tag_color(category);
    let bg = format!("color-mix(in srgb, {color} 12%, transparent)");
    rsx! {
        span {
            class: "patch-tag-pill",
            style: "color: {color}; background: {bg};",
            "{category}"
        }
    }
}

fn render_hero_update(hu: &HeroUpdate) -> Element {
    let ct_color = change_type_color(&hu.change_type);
    let ct_bg = change_type_bg(&hu.change_type);
    let ct_label = change_type_label(&hu.change_type);

    rsx! {
        div { class: "patch-hero-card",
            div { class: "patch-hero-card-header",
                span { class: "patch-hero-name", "{hu.hero_name}" }
                span {
                    class: "patch-change-badge",
                    style: "color: {ct_color}; background: {ct_bg};",
                    "{ct_label}"
                }
            }

            if let Some(comment) = &hu.dev_comment {
                p { class: "patch-dev-comment", "\"{comment}\"" }
            }

            ul { class: "patch-change-list",
                for change in hu.changes.iter() {
                    {render_change_item(change)}
                }
            }
        }
    }
}

fn render_change_item(change: &PatchChange) -> Element {
    rsx! {
        li { class: "patch-change-item",
            if let Some(ability) = &change.ability {
                span { class: "patch-change-ability", "{ability}: " }
            }
            "{change.description}"
        }
    }
}

fn render_section(section: &PatchSection) -> Element {
    if section.items.is_empty() {
        return rsx! {};
    }

    rsx! {
        h3 { class: "patch-section-title", "{section.category}" }
        ul { class: "patch-section-items",
            for item in section.items.iter() {
                li { class: "patch-section-item", "{item}" }
            }
        }
    }
}
