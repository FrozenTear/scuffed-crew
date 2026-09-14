use std::collections::HashMap;

use dioxus::prelude::*;
use serde::Deserialize;

use crate::hooks::use_api;

// --- Types ---

/// `{ "data": [...] }` envelope — same unwrap as Browse (`StrategyListResponse`).
/// `data` is required so a bare `[]` / `{}` cannot look like a successful empty list.
#[derive(Debug, Clone, Deserialize)]
struct ListResponse {
    #[serde(alias = "patches")]
    data: Vec<PatchNote>,
}

/// Local card type. Fields default so a thin API payload still deserializes;
/// unknown keys are ignored. Not a copy of any external patch-notes schema.
#[derive(Debug, Clone, Deserialize)]
struct PatchNote {
    #[serde(default)]
    version: String,
    #[serde(default, alias = "published_at")]
    date: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default, alias = "summary")]
    body: Option<String>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    hero_updates: Vec<HeroUpdate>,
    #[serde(default)]
    sections: Vec<PatchSection>,
}

#[derive(Debug, Clone, Deserialize)]
struct HeroUpdate {
    #[serde(default)]
    hero_id: String,
    hero_name: String,
    #[serde(default)]
    change_type: String,
    #[serde(default)]
    changes: Vec<PatchChange>,
    dev_comment: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct PatchChange {
    ability: Option<String>,
    description: String,
    #[serde(default)]
    change_type: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PatchSection {
    category: String,
    #[serde(default)]
    items: Vec<String>,
}

/// Distinguishes loading vs failed so a missing API cannot look like loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatchFetchState {
    Loading,
    Failed,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CardKind {
    Featured,
    Compact,
}

/// How many category pills a collapsed archive row may show before `+N`.
const COLLAPSED_TAG_LIMIT: usize = 2;

/// Sticky hero TOC only appears once an expanded patch has this many heroes.
const HERO_TOC_THRESHOLD: usize = 12;

/// `use_api` stores failure as `Some(None)` + `error`; still-in-flight is `None`.
fn classify_patch_fetch<T>(resource: Option<Option<&T>>, error: Option<&str>) -> PatchFetchState {
    match resource {
        None => {
            if error.is_some() {
                PatchFetchState::Failed
            } else {
                PatchFetchState::Loading
            }
        }
        Some(None) => PatchFetchState::Failed,
        Some(Some(_)) => PatchFetchState::Ready,
    }
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

fn patch_matches_filter(patch: &PatchNote, filter: &str) -> bool {
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

fn patch_matches_search(patch: &PatchNote, query: &str) -> bool {
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
    if let Some(body) = &patch.body
        && body.to_lowercase().contains(&q)
    {
        return true;
    }
    patch
        .hero_updates
        .iter()
        .any(|h| h.hero_name.to_lowercase().contains(&q))
}

fn patch_display_title(patch: &PatchNote) -> String {
    patch.title.clone().unwrap_or_else(|| {
        if patch.version.is_empty() {
            "Patch notes".to_string()
        } else {
            format!("Patch {}", patch.version)
        }
    })
}

fn patch_category_labels(patch: &PatchNote) -> Vec<String> {
    let mut categories = Vec::new();
    if !patch.hero_updates.is_empty() {
        categories.push("Hero Balance".to_string());
    }
    for section in &patch.sections {
        if !categories.iter().any(|c| c == &section.category) {
            categories.push(section.category.clone());
        }
    }
    categories
}

fn take_visible_tags(categories: &[String], limit: usize) -> (Vec<String>, usize) {
    if categories.len() <= limit {
        (categories.to_vec(), 0)
    } else {
        (categories[..limit].to_vec(), categories.len() - limit)
    }
}

fn year_from_date(date: &str) -> Option<&str> {
    let year = date.get(0..4)?;
    if year.bytes().all(|b| b.is_ascii_digit()) {
        Some(year)
    } else {
        None
    }
}

fn group_by_year<'a>(
    items: &[(usize, &'a PatchNote)],
) -> Vec<(String, Vec<(usize, &'a PatchNote)>)> {
    let mut groups: Vec<(String, Vec<(usize, &'a PatchNote)>)> = Vec::new();
    for &(idx, patch) in items {
        let year = year_from_date(&patch.date)
            .map(|y| y.to_string())
            .unwrap_or_else(|| "Undated".to_string());
        match groups.last_mut() {
            Some((existing, rows)) if *existing == year => rows.push((idx, patch)),
            _ => groups.push((year, vec![(idx, patch)])),
        }
    }
    groups
}

fn should_feature_latest(filter: &str, query: &str) -> bool {
    filter == "All" && query.trim().is_empty()
}

fn notes_count_label(visible: usize, total: usize, filtered: bool) -> String {
    if !filtered {
        if total == 1 {
            "1 note".to_string()
        } else {
            format!("{total} notes")
        }
    } else if visible == 1 {
        format!("1 of {total}")
    } else {
        format!("{visible} of {total}")
    }
}

fn hero_count_label(n: usize) -> String {
    if n == 1 {
        "1 hero".to_string()
    } else {
        format!("{n} heroes")
    }
}

fn hero_change_counts(updates: &[HeroUpdate]) -> Vec<(&'static str, usize)> {
    let mut buff = 0;
    let mut nerf = 0;
    let mut adj = 0;
    let mut bug = 0;
    let mut other = 0;
    for update in updates {
        match update.change_type.as_str() {
            "buff" => buff += 1,
            "nerf" => nerf += 1,
            "adjustment" => adj += 1,
            "bugfix" => bug += 1,
            _ => other += 1,
        }
    }
    let mut out = Vec::new();
    if buff > 0 {
        out.push(("Buff", buff));
    }
    if nerf > 0 {
        out.push(("Nerf", nerf));
    }
    if adj > 0 {
        out.push(("Adjustment", adj));
    }
    if bug > 0 {
        out.push(("Bug Fix", bug));
    }
    if other > 0 {
        out.push(("Other", other));
    }
    out
}

fn show_hero_toc(hero_count: usize) -> bool {
    hero_count >= HERO_TOC_THRESHOLD
}

/// Featured / newest patch (catalog index 0) starts expanded; archive stays closed.
fn initial_expanded_patches() -> Vec<usize> {
    vec![0]
}

fn toggle_open_hero(current: Option<usize>, clicked: usize) -> Option<usize> {
    if current == Some(clicked) {
        None
    } else {
        Some(clicked)
    }
}

fn apply_hero_toggle(
    map: &HashMap<usize, usize>,
    patch_idx: usize,
    hero_idx: usize,
) -> HashMap<usize, usize> {
    let mut next = map.clone();
    match toggle_open_hero(next.get(&patch_idx).copied(), hero_idx) {
        Some(open) => {
            next.insert(patch_idx, open);
        }
        None => {
            next.remove(&patch_idx);
        }
    }
    next
}

fn change_count_label(n: usize) -> String {
    if n == 1 {
        "1 change".to_string()
    } else {
        format!("{n} changes")
    }
}

fn change_marker(ct: &str) -> &'static str {
    match ct {
        "buff" => "\u{25B2}",
        "nerf" => "\u{25BC}",
        "adjustment" => "\u{25C6}",
        _ => "\u{2022}",
    }
}

fn filter_chip_compact_label(label: &str) -> &str {
    match label {
        "Competitive" => "Comp",
        _ => label,
    }
}

fn hero_dom_id(patch_idx: usize, hero_idx: usize) -> String {
    format!("patch-{patch_idx}-hero-{hero_idx}")
}

fn scroll_to_hero_card(dom_id: &str) {
    #[cfg(target_arch = "wasm32")]
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(dom_id))
    {
        el.scroll_into_view();
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = dom_id;
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

fn summary_badge_color(label: &str) -> &'static str {
    match label {
        "Buff" => change_type_color("buff"),
        "Nerf" => change_type_color("nerf"),
        "Adjustment" => change_type_color("adjustment"),
        "Bug Fix" => change_type_color("bugfix"),
        _ => change_type_color(""),
    }
}

fn summary_badge_bg(label: &str) -> &'static str {
    match label {
        "Buff" => change_type_bg("buff"),
        "Nerf" => change_type_bg("nerf"),
        "Adjustment" => change_type_bg("adjustment"),
        "Bug Fix" => change_type_bg("bugfix"),
        _ => change_type_bg(""),
    }
}

// --- CSS ---

const PAGE_CSS: &str = r#"
    .patch-page {
        padding: var(--space-6) var(--space-6) var(--space-12);
        max-width: 1200px;
        margin: 0 auto;
    }
    .patch-header {
        display: flex;
        align-items: baseline;
        justify-content: space-between;
        gap: 0.75rem;
        margin-bottom: 0.85rem;
        flex-wrap: wrap;
    }
    .patch-page-title {
        font-family: var(--font-head);
        font-size: 1.85rem;
        color: var(--text);
        letter-spacing: 0.12em;
        text-transform: uppercase;
        margin: 0;
        line-height: 1.1;
    }
    .patch-count {
        font-family: var(--font-mono);
        font-size: 0.7rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        color: var(--text-3);
    }
    .patch-toolbar {
        position: sticky;
        /* Public site-nav is 48px fixed. Strategy nav is 50px in-flow, so stick at 0. */
        top: 48px;
        z-index: 8;
        display: flex;
        flex-direction: column;
        gap: 0.65rem;
        margin: 0 -0.35rem 1.15rem;
        padding: 0.65rem 0.35rem 0.75rem;
        background: color-mix(in srgb, var(--bg) 88%, transparent);
        backdrop-filter: blur(14px);
        border-bottom: 1px solid var(--border);
    }
    .patch-toolbar-row {
        display: flex;
        align-items: center;
        gap: 0.65rem;
    }
    .patch-search {
        flex: 1;
        min-width: 0;
        padding: 0.45rem 0.75rem;
        border-radius: var(--radius-sm);
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
    .patch-text-btn {
        flex-shrink: 0;
        border: none;
        background: none;
        color: var(--text-3);
        font-family: var(--font-body);
        font-size: 0.75rem;
        cursor: pointer;
        padding: 0.25rem 0;
        text-decoration: underline;
        text-underline-offset: 2px;
    }
    .patch-text-btn:hover {
        color: var(--text);
    }
    .patch-filters {
        display: flex;
        gap: 0.4rem;
        flex-wrap: wrap;
    }
    .patch-chip {
        padding: 0.28rem 0.65rem;
        border-radius: var(--radius-pill);
        font-size: 0.7rem;
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
    .patch-year-block + .patch-year-block {
        margin-top: 1.15rem;
    }
    .patch-year {
        font-family: var(--font-mono);
        font-size: 0.68rem;
        font-weight: 600;
        letter-spacing: 0.14em;
        text-transform: uppercase;
        color: var(--text-3);
        margin: 0 0 0.45rem;
    }
    .patch-featured-wrap {
        margin-bottom: 1.35rem;
    }
    .patch-timeline {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
    }
    .patch-card {
        background: var(--surface);
        background-image: none;
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        overflow: hidden;
        transition: border-color 0.2s;
    }
    .patch-card:hover {
        border-color: var(--accent-soft);
    }
    .patch-card-featured {
        border-color: color-mix(in srgb, var(--accent) 35%, var(--border));
        background:
            linear-gradient(
                180deg,
                color-mix(in srgb, var(--accent) 7%, var(--surface)) 0%,
                var(--surface) 42%
            );
    }
    .patch-card-header {
        display: grid;
        grid-template-columns: auto minmax(0, 1fr) auto auto auto;
        align-items: center;
        gap: 0.55rem 0.65rem;
        width: 100%;
        padding: 0.55rem 0.85rem;
        border: none;
        background: transparent;
        color: inherit;
        font: inherit;
        text-align: left;
        cursor: pointer;
        user-select: none;
    }
    .patch-card-header:hover {
        background: var(--surface-2);
    }
    .patch-card-featured .patch-card-header {
        grid-template-columns: auto minmax(0, 1fr) auto;
        align-items: start;
        padding: 1rem 1.1rem 0.75rem;
        gap: 0.65rem 0.85rem;
    }
    .patch-card-featured .patch-card-header:hover {
        background: transparent;
    }
    .patch-version-badge {
        font-family: var(--font-mono);
        font-size: 0.72rem;
        font-weight: 600;
        padding: 0.12rem 0.45rem;
        border-radius: 4px;
        background: var(--accent-soft);
        color: var(--accent);
        flex-shrink: 0;
    }
    .patch-card-title {
        font-family: var(--font-head);
        font-weight: 700;
        font-size: 0.9rem;
        color: var(--text);
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .patch-card-featured .patch-card-title {
        font-size: 1.2rem;
        letter-spacing: 0.02em;
        white-space: normal;
        overflow: visible;
        line-height: 1.25;
    }
    .patch-card-date {
        font-family: var(--font-mono);
        font-size: 0.7rem;
        color: var(--text-3);
        white-space: nowrap;
    }
    .patch-hero-count {
        font-size: 0.68rem;
        color: var(--text-2);
        background: var(--surface-2);
        padding: 0.08rem 0.4rem;
        border-radius: var(--radius-pill);
        white-space: nowrap;
    }
    .patch-tag-pills {
        display: flex;
        gap: 0.3rem;
        flex-wrap: nowrap;
        align-items: center;
        min-width: 0;
        overflow: hidden;
    }
    .patch-card-featured .patch-tag-pills {
        flex-wrap: wrap;
        overflow: visible;
        grid-column: 2 / -2;
    }
    .patch-tag-pill {
        font-size: 0.6rem;
        padding: 0.08rem 0.4rem;
        border-radius: var(--radius-pill);
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.03em;
        white-space: nowrap;
    }
    .patch-tag-more {
        font-family: var(--font-mono);
        font-size: 0.6rem;
        color: var(--text-3);
        white-space: nowrap;
    }
    .patch-expand-icon {
        font-size: 0.65rem;
        color: var(--text-3);
        line-height: 1;
        transition: transform 0.2s;
    }
    .patch-expand-icon.open {
        transform: rotate(180deg);
    }
    .patch-featured-copy {
        padding: 0 1.1rem 0.85rem;
    }
    .patch-card-body {
        padding: 0 1.1rem 1.1rem;
        border-top: 1px solid var(--border);
    }
    .patch-body-lead {
        font-size: 0.85rem;
        color: var(--text-2);
        line-height: 1.55;
        margin: 0.85rem 0 0.15rem;
    }
    .patch-body-lead.clamp {
        display: -webkit-box;
        -webkit-line-clamp: 3;
        -webkit-box-orient: vertical;
        overflow: hidden;
        margin: 0;
    }
    .patch-change-summary {
        display: flex;
        flex-wrap: wrap;
        gap: 0.35rem;
        margin-top: 0.65rem;
    }
    .patch-section-title {
        font-family: var(--font-head);
        font-weight: 700;
        font-size: 0.8rem;
        color: var(--text);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        margin: 1rem 0 0.5rem;
    }
    .patch-hero-layout {
        display: grid;
        grid-template-columns: 11rem minmax(0, 1fr);
        gap: 0.85rem 1rem;
        align-items: start;
    }
    .patch-hero-stack {
        display: block;
    }
    .patch-hero-toc {
        position: sticky;
        top: calc(48px + 5.5rem);
        display: flex;
        flex-direction: column;
        gap: 0.28rem;
        max-height: calc(100vh - 8.5rem);
        overflow-y: auto;
        padding-right: 0.15rem;
    }
    .patch-hero-toc-title {
        font-family: var(--font-mono);
        font-size: 0.62rem;
        font-weight: 600;
        letter-spacing: 0.12em;
        text-transform: uppercase;
        color: var(--text-3);
        margin: 0 0 0.15rem;
    }
    .patch-hero-toc-item {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 0.35rem;
        width: 100%;
        border: 1px solid transparent;
        border-radius: var(--radius-sm);
        background: transparent;
        color: var(--text-2);
        font: inherit;
        font-size: 0.75rem;
        text-align: left;
        padding: 0.28rem 0.45rem;
        cursor: pointer;
    }
    .patch-hero-toc-item:hover {
        background: var(--surface-2);
        color: var(--text);
    }
    .patch-hero-toc-item.active {
        border-color: var(--accent-soft);
        background: var(--accent-soft);
        color: var(--accent);
    }
    .patch-hero-grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(min(340px, 100%), 1fr));
        gap: 10px;
    }
    .patch-hero-card {
        background: var(--surface-2);
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        overflow: hidden;
    }
    .patch-hero-card.open {
        grid-column: 1 / -1;
        border-color: color-mix(in srgb, var(--accent) 28%, var(--border));
    }
    .patch-hero-card-header {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        width: 100%;
        margin: 0;
        padding: 0.55rem 0.75rem;
        border: none;
        background: transparent;
        color: inherit;
        font: inherit;
        text-align: left;
        cursor: pointer;
    }
    .patch-hero-card-header:hover {
        background: color-mix(in srgb, var(--surface) 55%, var(--surface-2));
    }
    .patch-hero-name {
        font-family: var(--font-head);
        font-weight: 700;
        font-size: 0.88rem;
        color: var(--text);
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .patch-hero-meta {
        margin-left: auto;
        display: flex;
        align-items: center;
        gap: 0.4rem;
        flex-shrink: 0;
    }
    .patch-hero-change-count {
        font-family: var(--font-mono);
        font-size: 0.62rem;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: var(--text-3);
        white-space: nowrap;
    }
    .patch-change-badge {
        font-size: 0.6rem;
        padding: 0.1rem 0.45rem;
        border-radius: var(--radius-pill);
        font-weight: 700;
        text-transform: uppercase;
        letter-spacing: 0.04em;
        white-space: nowrap;
    }
    .patch-hero-details {
        padding: 0 0.75rem 0.75rem;
        border-top: 1px solid var(--border);
    }
    .patch-dev-comment {
        font-size: 0.78rem;
        color: var(--text-2);
        font-style: italic;
        padding: 0.45rem 0.7rem;
        margin: 0.55rem 0 0.5rem;
        border-left: 2px solid var(--border);
        line-height: 1.5;
    }
    .patch-change-list {
        list-style: none;
        padding: 0;
        margin: 0.15rem 0 0;
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(min(280px, 100%), 1fr));
        gap: 0.35rem 1rem;
    }
    .patch-change-item {
        font-size: 0.8rem;
        color: var(--text);
        line-height: 1.5;
        display: flex;
        align-items: flex-start;
        gap: 0.4rem;
    }
    .patch-change-marker {
        flex-shrink: 0;
        font-size: 0.68rem;
        line-height: 1.6;
        font-weight: 700;
    }
    .patch-change-copy {
        min-width: 0;
    }
    .patch-change-ability {
        font-weight: 700;
        color: var(--text);
    }
    .patch-section-items {
        list-style: none;
        padding: 0;
        margin: 0;
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(min(280px, 100%), 1fr));
        gap: 0.25rem 1.15rem;
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
        padding: 4rem 1rem;
        font-size: 0.95rem;
    }
    .patch-empty-hint {
        margin-top: 0.5rem;
        font-size: 0.85rem;
    }
    .patch-error {
        color: var(--danger);
    }
    .patch-retry {
        margin-top: 1rem;
    }
    .patch-chip-short {
        display: none;
    }
    @media (max-width: 720px) {
        .patch-page {
            padding: var(--space-4) var(--space-4) var(--space-8);
        }
        .patch-page-title {
            font-size: 1.45rem;
            letter-spacing: 0.08em;
        }
        .patch-toolbar {
            margin-left: -0.15rem;
            margin-right: -0.15rem;
        }
        .patch-filters {
            flex-wrap: nowrap;
            overflow-x: auto;
            padding-bottom: 0.15rem;
            -webkit-overflow-scrolling: touch;
            scrollbar-width: none;
        }
        .patch-filters::-webkit-scrollbar {
            display: none;
        }
        .patch-chip {
            flex-shrink: 0;
        }
        .patch-chip-full {
            display: none;
        }
        .patch-chip-short {
            display: inline;
        }
        .patch-hero-layout {
            grid-template-columns: 1fr;
        }
        .patch-hero-toc {
            position: static;
            flex-direction: row;
            flex-wrap: nowrap;
            max-height: none;
            overflow-x: auto;
            overflow-y: hidden;
            padding-right: 0;
            padding-bottom: 0.2rem;
            -webkit-overflow-scrolling: touch;
            scrollbar-width: none;
        }
        .patch-hero-toc::-webkit-scrollbar {
            display: none;
        }
        .patch-hero-toc-title {
            display: none;
        }
        .patch-hero-toc-item {
            width: auto;
            flex-shrink: 0;
        }
        .patch-card-header {
            grid-template-columns: auto minmax(0, 1fr) auto auto;
        }
        .patch-card-date,
        .patch-tag-pills {
            display: none;
        }
        .patch-card-featured .patch-card-header {
            grid-template-columns: auto minmax(0, 1fr) auto;
        }
        .patch-card-featured .patch-card-date,
        .patch-card-featured .patch-tag-pills {
            display: flex;
        }
        .patch-card-featured .patch-card-title {
            font-size: 1.05rem;
        }
    }
    /* StrategyLayout nav is 50px in document flow (not fixed). */
    [data-accent="strategy"] .patch-toolbar {
        top: 0;
    }
    [data-accent="strategy"] .patch-hero-toc {
        top: 5.5rem;
    }
"#;

// --- Component ---

/// Shared Patch Notes UI — used by `/patch-notes` (PublicLayout) and
/// `/strategy/patch-notes` (StrategyLayout). Same fetch + unwrap.
#[component]
pub fn PatchNotesPage() -> Element {
    let patches = use_api::<ListResponse>("/api/strategy/patch-notes");

    let mut search_query = use_signal(String::new);
    let mut active_filter = use_signal(|| "All".to_string());
    let mut expanded: Signal<Vec<usize>> = use_signal(initial_expanded_patches);
    let mut open_hero: Signal<HashMap<usize, usize>> = use_signal(HashMap::new);

    rsx! {
        style { {PAGE_CSS} }

        div { class: "patch-page",
            {
                let data = patches.data.read();
                let inner = data.as_ref().and_then(|d| d.as_ref());
                let err = patches.error.read().as_ref().cloned();
                let state = classify_patch_fetch(data.as_ref().map(|d| d.as_ref()), err.as_deref());
                let total = inner.map(|r| r.data.len()).unwrap_or(0);
                let query = (search_query)();
                let filter = (active_filter)();
                let filtered = filter != "All" || !query.trim().is_empty();
                let visible_len = inner
                    .map(|resp| {
                        resp.data
                            .iter()
                            .filter(|p| patch_matches_search(p, &query) && patch_matches_filter(p, &filter))
                            .count()
                    })
                    .unwrap_or(0);
                let count_label = notes_count_label(visible_len, total, filtered);
                let show_toolbar = matches!(state, PatchFetchState::Ready) && total > 0;

                rsx! {
                    div { class: "patch-header",
                        h1 { class: "patch-page-title", "Patch Notes" }
                        if show_toolbar {
                            span { class: "patch-count", "{count_label}" }
                        }
                    }

                    if show_toolbar {
                        div { class: "patch-toolbar",
                            div { class: "patch-toolbar-row",
                                input {
                                    class: "patch-search",
                                    r#type: "search",
                                    placeholder: "Search version, title, or hero...",
                                    value: "{search_query}",
                                    oninput: move |evt| search_query.set(evt.value()),
                                }
                                if visible_len > 1 {
                                    {
                                        let resp_indices: Vec<usize> = inner
                                            .map(|resp| {
                                                resp.data
                                                    .iter()
                                                    .enumerate()
                                                    .filter(|(_, p)| {
                                                        patch_matches_search(p, &query)
                                                            && patch_matches_filter(p, &filter)
                                                    })
                                                    .map(|(i, _)| i)
                                                    .collect()
                                            })
                                            .unwrap_or_default();
                                        let opened = (expanded)();
                                        let all_open = !resp_indices.is_empty()
                                            && resp_indices.iter().all(|i| opened.contains(i));
                                        let label = if all_open { "Collapse all" } else { "Expand all" };
                                        rsx! {
                                            button {
                                                class: "patch-text-btn",
                                                onclick: move |_| {
                                                    let opened_now = (expanded)();
                                                    if resp_indices.iter().all(|i| opened_now.contains(i)) {
                                                        expanded.set(Vec::new());
                                                    } else {
                                                        expanded.set(resp_indices.clone());
                                                    }
                                                },
                                                "{label}"
                                            }
                                        }
                                    }
                                }
                            }
                            div { class: "patch-filters",
                                for label in FILTER_OPTIONS {
                                    {render_filter_chip(label, &filter, &mut active_filter)}
                                }
                            }
                        }
                    }

                    {
                        match (state, inner) {
                            (PatchFetchState::Loading, _) => rsx! {
                                p { class: "patch-loading", "Loading patch notes..." }
                            },
                            (PatchFetchState::Failed, _) => {
                                let message = err.unwrap_or_else(|| "Request failed".to_string());
                                let mut refresh = patches.refresh;
                                rsx! {
                                    div { class: "patch-empty",
                                        p { class: "patch-error",
                                            "Failed to load patch notes: {message}"
                                        }
                                        button {
                                            class: "patch-chip patch-retry",
                                            onclick: move |_| refresh += 1,
                                            "Retry"
                                        }
                                    }
                                }
                            },
                            (PatchFetchState::Ready, Some(resp)) if resp.data.is_empty() => rsx! {
                                div { class: "patch-empty",
                                    p { "No patch notes yet." }
                                    p { class: "patch-empty-hint",
                                        "When the catalog is published they will show up here."
                                    }
                                }
                            },
                            (PatchFetchState::Ready, Some(resp)) => {
                                let visible: Vec<(usize, &PatchNote)> = resp.data
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, p)| {
                                        patch_matches_search(p, &query) && patch_matches_filter(p, &filter)
                                    })
                                    .collect();

                                if visible.is_empty() {
                                    rsx! {
                                        div { class: "patch-empty",
                                            p { "No patch notes match these filters." }
                                            p { class: "patch-empty-hint",
                                                "Try clearing search or switching the category filter."
                                            }
                                        }
                                    }
                                } else {
                                    let expanded_indices = (expanded)();
                                    let feature = should_feature_latest(&filter, &query);
                                    let featured = if feature { visible.first().copied() } else { None };
                                    let archive = if featured.is_some() { &visible[1..] } else { visible.as_slice() };
                                    let groups = group_by_year(archive);

                                    rsx! {
                                        if let Some((idx, patch)) = featured {
                                            div { class: "patch-featured-wrap",
                                                p { class: "patch-year", "Latest" }
                                                {render_patch_card(
                                                    idx,
                                                    patch,
                                                    expanded_indices.contains(&idx),
                                                    CardKind::Featured,
                                                    &mut expanded,
                                                    &mut open_hero,
                                                )}
                                            }
                                        }
                                        for (year, rows) in groups {
                                            div { class: "patch-year-block",
                                                p { class: "patch-year", "{year}" }
                                                div { class: "patch-timeline",
                                                    for (idx, patch) in rows.iter() {
                                                        {render_patch_card(
                                                            *idx,
                                                            patch,
                                                            expanded_indices.contains(idx),
                                                            CardKind::Compact,
                                                            &mut expanded,
                                                            &mut open_hero,
                                                        )}
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            (PatchFetchState::Ready, None) => rsx! {
                                div { class: "patch-empty",
                                    p { "No patch notes yet." }
                                }
                            },
                        }
                    }
                }
            }
        }
    }
}

/// Public-site route (`/patch-notes` under PublicLayout).
#[component]
pub fn PatchNotes() -> Element {
    rsx! { PatchNotesPage {} }
}

/// Strategy-section route (`/strategy/patch-notes` under StrategyLayout).
#[component]
pub fn StrategyPatchNotes() -> Element {
    rsx! { PatchNotesPage {} }
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
    let compact = filter_chip_compact_label(label).to_string();
    let mut sig = *signal;

    rsx! {
        button {
            class: "{class}",
            title: "{label}",
            onclick: move |_| sig.set(label_owned.clone()),
            span { class: "patch-chip-full", "{label}" }
            span { class: "patch-chip-short", "{compact}" }
        }
    }
}

fn render_patch_card(
    idx: usize,
    patch: &PatchNote,
    is_expanded: bool,
    kind: CardKind,
    expanded_signal: &mut Signal<Vec<usize>>,
    open_hero: &mut Signal<HashMap<usize, usize>>,
) -> Element {
    let title = patch_display_title(patch);
    let hero_count = patch.hero_updates.len();
    let expand_class = if is_expanded {
        "patch-expand-icon open"
    } else {
        "patch-expand-icon"
    };
    let card_class = match kind {
        CardKind::Featured => "patch-card patch-card-featured",
        CardKind::Compact => "patch-card",
    };
    let tag_limit = match kind {
        CardKind::Featured => usize::MAX,
        CardKind::Compact => COLLAPSED_TAG_LIMIT,
    };
    let categories = patch_category_labels(patch);
    let (visible_tags, overflow) = take_visible_tags(&categories, tag_limit);
    let change_summary = hero_change_counts(&patch.hero_updates);
    let version = patch.version.clone();
    let date = patch.date.clone();
    let url = patch.url.clone();
    let body = patch.body.clone();
    let hero_updates = patch.hero_updates.clone();
    let sections = patch.sections.clone();
    let featured = kind == CardKind::Featured;
    let show_lead_preview = featured && !is_expanded;
    let open_hero_idx = (*open_hero)().get(&idx).copied();
    let mut sig = *expanded_signal;
    let mut hero_sig = *open_hero;

    rsx! {
        div { class: "{card_class}",
            button {
                class: "patch-card-header",
                r#type: "button",
                aria_expanded: if is_expanded { "true" } else { "false" },
                onclick: move |_| {
                    let mut current = sig();
                    if let Some(pos) = current.iter().position(|&i| i == idx) {
                        current.remove(pos);
                    } else {
                        current.push(idx);
                    }
                    sig.set(current);
                },

                if !version.is_empty() {
                    span { class: "patch-version-badge", "{version}" }
                }
                span { class: "patch-card-title", "{title}" }
                if !date.is_empty() {
                    span { class: "patch-card-date", "{date}" }
                }
                if hero_count > 0 {
                    {
                        let label = hero_count_label(hero_count);
                        rsx! { span { class: "patch-hero-count", "{label}" } }
                    }
                }
                div { class: "patch-tag-pills",
                    for cat in visible_tags.iter() {
                        {render_tag_pill(cat)}
                    }
                    if overflow > 0 {
                        span { class: "patch-tag-more", "+{overflow}" }
                    }
                }
                span { class: "{expand_class}", "\u{25bc}" }
            }

            if show_lead_preview {
                div { class: "patch-featured-copy",
                    if let Some(lead) = body.as_ref().filter(|s| !s.is_empty()) {
                        p { class: "patch-body-lead clamp", "{lead}" }
                    }
                    if !change_summary.is_empty() {
                        div { class: "patch-change-summary",
                            for (label, count) in change_summary.iter() {
                                {
                                    let color = summary_badge_color(label);
                                    let bg = summary_badge_bg(label);
                                    rsx! {
                                        span {
                                            class: "patch-change-badge",
                                            style: "color: {color}; background: {bg};",
                                            "{count} {label}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if is_expanded {
                div { class: "patch-card-body",
                    if let Some(lead) = body.as_ref().filter(|s| !s.is_empty()) {
                        p { class: "patch-body-lead clamp", "{lead}" }
                    }

                    if !hero_updates.is_empty() {
                        h3 { class: "patch-section-title", "Hero Balance" }
                        {
                            let toc = show_hero_toc(hero_updates.len());
                            let layout_class = if toc {
                                "patch-hero-layout"
                            } else {
                                "patch-hero-stack"
                            };
                            rsx! {
                                div { class: "{layout_class}",
                                    if toc {
                                        nav {
                                            class: "patch-hero-toc",
                                            aria_label: "Heroes in this patch",
                                            p { class: "patch-hero-toc-title", "Heroes" }
                                            for (hi, hu) in hero_updates.iter().enumerate() {
                                                {render_toc_item(
                                                    idx,
                                                    hi,
                                                    hu,
                                                    open_hero_idx == Some(hi),
                                                    &mut hero_sig,
                                                )}
                                            }
                                        }
                                    }
                                    div { class: "patch-hero-grid",
                                        for (hi, hu) in hero_updates.iter().enumerate() {
                                            {render_hero_update(
                                                idx,
                                                hi,
                                                hu,
                                                open_hero_idx == Some(hi),
                                                &mut hero_sig,
                                            )}
                                        }
                                    }
                                }
                            }
                        }
                    }

                    for section in sections.iter() {
                        {render_section(section)}
                    }

                    if !url.is_empty() {
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

fn render_toc_item(
    patch_idx: usize,
    hero_idx: usize,
    hu: &HeroUpdate,
    is_open: bool,
    open_hero: &mut Signal<HashMap<usize, usize>>,
) -> Element {
    let class = if is_open {
        "patch-hero-toc-item active"
    } else {
        "patch-hero-toc-item"
    };
    let name = hu.hero_name.clone();
    let mut sig = *open_hero;

    rsx! {
        button {
            class: "{class}",
            r#type: "button",
            aria_current: if is_open { "true" } else { "false" },
            onclick: move |_| {
                let opening = (*sig)().get(&patch_idx).copied() != Some(hero_idx);
                sig.set(apply_hero_toggle(&sig(), patch_idx, hero_idx));
                if opening {
                    scroll_to_hero_card(&hero_dom_id(patch_idx, hero_idx));
                }
            },
            "{name}"
        }
    }
}

fn render_hero_update(
    patch_idx: usize,
    hero_idx: usize,
    hu: &HeroUpdate,
    is_open: bool,
    open_hero: &mut Signal<HashMap<usize, usize>>,
) -> Element {
    let ct_color = change_type_color(&hu.change_type);
    let ct_bg = change_type_bg(&hu.change_type);
    let ct_label = change_type_label(&hu.change_type);
    let count_label = change_count_label(hu.changes.len());
    let card_class = if is_open {
        "patch-hero-card open"
    } else {
        "patch-hero-card"
    };
    let expand_class = if is_open {
        "patch-expand-icon open"
    } else {
        "patch-expand-icon"
    };
    let dom_id = hero_dom_id(patch_idx, hero_idx);
    let hero_name = hu.hero_name.clone();
    let mut sig = *open_hero;

    rsx! {
        div {
            class: "{card_class}",
            id: "{dom_id}",
            button {
                class: "patch-hero-card-header",
                r#type: "button",
                aria_expanded: if is_open { "true" } else { "false" },
                onclick: move |_| {
                    let opening = (*sig)().get(&patch_idx).copied() != Some(hero_idx);
                    sig.set(apply_hero_toggle(&sig(), patch_idx, hero_idx));
                    if opening {
                        scroll_to_hero_card(&hero_dom_id(patch_idx, hero_idx));
                    }
                },
                span { class: "patch-hero-name", "{hero_name}" }
                span {
                    class: "patch-change-badge",
                    style: "color: {ct_color}; background: {ct_bg};",
                    "{ct_label}"
                }
                div { class: "patch-hero-meta",
                    span { class: "patch-hero-change-count", "{count_label}" }
                    span { class: "{expand_class}", "\u{25bc}" }
                }
            }

            if is_open {
                div { class: "patch-hero-details",
                    if let Some(comment) = &hu.dev_comment {
                        p { class: "patch-dev-comment", "\"{comment}\"" }
                    }

                    if !hu.changes.is_empty() {
                        ul { class: "patch-change-list",
                            for change in hu.changes.iter() {
                                {render_change_item(change)}
                            }
                        }
                    }
                }
            }
        }
    }
}

fn render_change_item(change: &PatchChange) -> Element {
    let marker = change_marker(&change.change_type);
    let marker_color = change_type_color(&change.change_type);

    rsx! {
        li { class: "patch-change-item",
            span {
                class: "patch-change-marker",
                style: "color: {marker_color};",
                aria_hidden: "true",
                "{marker}"
            }
            span { class: "patch-change-copy",
                if let Some(ability) = &change.ability {
                    span { class: "patch-change-ability", "{ability}: " }
                }
                "{change.description}"
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn note(
        version: &str,
        date: &str,
        title: Option<&str>,
        heroes: Vec<HeroUpdate>,
        sections: Vec<PatchSection>,
    ) -> PatchNote {
        PatchNote {
            version: version.into(),
            date: date.into(),
            title: title.map(str::to_string),
            body: None,
            url: String::new(),
            hero_updates: heroes,
            sections,
        }
    }

    fn hero(name: &str, change_type: &str) -> HeroUpdate {
        HeroUpdate {
            hero_id: name.to_lowercase(),
            hero_name: name.into(),
            change_type: change_type.into(),
            changes: vec![],
            dev_comment: None,
        }
    }

    #[test]
    fn failed_fetch_is_not_loading() {
        // Old bug: `.ok()` + `None => Loading` treated Err as eternal spinner.
        assert_eq!(
            classify_patch_fetch::<ListResponse>(Some(None), Some("HTTP error 404: Not Found")),
            PatchFetchState::Failed
        );
        assert_eq!(
            classify_patch_fetch::<ListResponse>(None, None),
            PatchFetchState::Loading
        );
        let ready = ListResponse { data: vec![] };
        assert_eq!(
            classify_patch_fetch(Some(Some(&ready)), None),
            PatchFetchState::Ready
        );
    }

    #[test]
    fn list_envelope_unwraps_data_not_bare_vec() {
        let empty: ListResponse = serde_json::from_str(r#"{"data":[]}"#).unwrap();
        assert!(empty.data.is_empty());

        let aliased: ListResponse = serde_json::from_str(r#"{"patches":[]}"#).unwrap();
        assert!(aliased.data.is_empty());

        let empty_obj: Result<ListResponse, _> = serde_json::from_str("{}");
        assert!(empty_obj.is_err(), "missing data key is not an empty list");

        let bare_empty: Result<ListResponse, _> = serde_json::from_str("[]");
        assert!(
            bare_empty.is_err(),
            "GET /api/strategy/patch-notes is the list envelope, not a bare Vec"
        );

        let bare_items: Result<ListResponse, _> =
            serde_json::from_str(r#"[{"version":"1","date":"2026-01-01"}]"#);
        assert!(
            bare_items.is_err(),
            "a populated bare Vec must not deserialize as the envelope"
        );
    }

    #[test]
    fn list_envelope_parses_version_title_date_sections() {
        let json = r#"{
            "data": [{
                "version": "2.15",
                "date": "2026-09-01",
                "title": "Season 18 Mid-Season",
                "body": "This is a mid-season balance update.",
                "url": "https://overwatch.blizzard.com/news/patch-notes",
                "hero_updates": [{
                    "hero_id": "ana",
                    "hero_name": "Ana",
                    "change_type": "buff",
                    "changes": [{"ability": "Biotic Rifle", "description": "Damage increased"}]
                }],
                "sections": [{"category": "Bug Fixes", "items": ["Fixed a crash"]}]
            }]
        }"#;
        let resp: ListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data.len(), 1);
        assert_eq!(resp.data[0].version, "2.15");
        assert_eq!(resp.data[0].title.as_deref(), Some("Season 18 Mid-Season"));
        assert_eq!(resp.data[0].date, "2026-09-01");
        assert_eq!(
            resp.data[0].body.as_deref(),
            Some("This is a mid-season balance update.")
        );
        assert_eq!(resp.data[0].hero_updates.len(), 1);
        assert_eq!(resp.data[0].sections[0].category, "Bug Fixes");
    }

    #[test]
    fn missing_optional_collections_default_empty() {
        let json = r#"{"data":[{"version":"1.0","date":"2026-01-01"}]}"#;
        let resp: ListResponse = serde_json::from_str(json).unwrap();
        assert!(resp.data[0].hero_updates.is_empty());
        assert!(resp.data[0].sections.is_empty());
        assert!(resp.data[0].url.is_empty());
        assert!(resp.data[0].body.is_none());
    }

    #[test]
    fn filter_hero_balance_requires_hero_updates() {
        let with_heroes = PatchNote {
            version: "1".into(),
            date: "2026-01-01".into(),
            title: None,
            body: Some("Hotfix for console aim assist.".into()),
            url: String::new(),
            hero_updates: vec![hero("Ana", "buff")],
            sections: vec![],
        };
        let without = PatchNote {
            hero_updates: vec![],
            ..with_heroes.clone()
        };
        assert!(patch_matches_filter(&with_heroes, "Hero Balance"));
        assert!(!patch_matches_filter(&without, "Hero Balance"));
        assert!(patch_matches_search(&with_heroes, "ana"));
        assert!(!patch_matches_search(&with_heroes, "junkrat"));
        assert!(patch_matches_search(&with_heroes, "console"));
    }

    #[test]
    fn summary_alias_fills_body() {
        let json = r#"{"data":[{"version":"1","date":"2026-08-11","summary":"Season launch."}]}"#;
        let resp: ListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data[0].body.as_deref(), Some("Season launch."));
    }

    #[test]
    fn thin_core_fields_deserializes() {
        let json = r#"{
            "data": [{
                "version": "2.18.1",
                "title": "Mid-season balance",
                "date": "2026-08-20",
                "body": "This is a mid-season balance update."
            }]
        }"#;
        let resp: ListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data[0].version, "2.18.1");
        assert_eq!(resp.data[0].title.as_deref(), Some("Mid-season balance"));
        assert_eq!(resp.data[0].date, "2026-08-20");
        assert_eq!(
            resp.data[0].body.as_deref(),
            Some("This is a mid-season balance update.")
        );
        assert!(resp.data[0].hero_updates.is_empty());
        assert!(resp.data[0].sections.is_empty());
    }

    #[test]
    fn missing_version_and_published_at_alias_are_ok() {
        let json =
            r#"{"data":[{"title":"Hotfix","published_at":"2026-08-12","body":"Client update."}]}"#;
        let resp: ListResponse = serde_json::from_str(json).unwrap();
        assert!(resp.data[0].version.is_empty());
        assert_eq!(resp.data[0].date, "2026-08-12");
        assert_eq!(resp.data[0].title.as_deref(), Some("Hotfix"));
        assert_eq!(resp.data[0].body.as_deref(), Some("Client update."));
    }

    #[test]
    fn year_from_date_reads_yyyy_prefix() {
        assert_eq!(year_from_date("2026-09-01"), Some("2026"));
        assert_eq!(year_from_date("2025"), Some("2025"));
        assert_eq!(year_from_date("hotfix"), None);
        assert_eq!(year_from_date(""), None);
    }

    #[test]
    fn group_by_year_keeps_catalog_order() {
        let a = note("3", "2026-09-01", Some("Sep"), vec![], vec![]);
        let b = note("2", "2026-01-01", Some("Jan"), vec![], vec![]);
        let c = note("1", "2025-12-01", Some("Dec"), vec![], vec![]);
        let items = vec![(0, &a), (1, &b), (2, &c)];
        let groups = group_by_year(&items);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "2026");
        assert_eq!(groups[0].1.len(), 2);
        assert_eq!(groups[1].0, "2025");
        assert_eq!(groups[1].1[0].0, 2);
    }

    #[test]
    fn collapsed_tags_cap_and_overflow() {
        let labels = vec![
            "Hero Balance".into(),
            "Bug Fixes".into(),
            "Maps".into(),
            "Competitive".into(),
        ];
        let (shown, extra) = take_visible_tags(&labels, COLLAPSED_TAG_LIMIT);
        assert_eq!(shown, ["Hero Balance", "Bug Fixes"]);
        assert_eq!(extra, 2);
        let (all, none) = take_visible_tags(&labels, usize::MAX);
        assert_eq!(all.len(), 4);
        assert_eq!(none, 0);
    }

    #[test]
    fn category_labels_include_hero_balance() {
        let patch = note(
            "1",
            "2026-01-01",
            None,
            vec![hero("Ana", "buff")],
            vec![PatchSection {
                category: "Bug Fixes".into(),
                items: vec!["fix".into()],
            }],
        );
        assert_eq!(patch_category_labels(&patch), ["Hero Balance", "Bug Fixes"]);
    }

    #[test]
    fn feature_latest_only_on_unfiltered_all() {
        assert!(should_feature_latest("All", ""));
        assert!(should_feature_latest("All", "   "));
        assert!(!should_feature_latest("Hero Balance", ""));
        assert!(!should_feature_latest("All", "ana"));
    }

    #[test]
    fn notes_count_label_reports_filter_state() {
        assert_eq!(notes_count_label(35, 35, false), "35 notes");
        assert_eq!(notes_count_label(1, 1, false), "1 note");
        assert_eq!(notes_count_label(4, 35, true), "4 of 35");
        assert_eq!(notes_count_label(1, 35, true), "1 of 35");
    }

    #[test]
    fn hero_change_counts_skips_empty_buckets() {
        let updates = vec![
            hero("Ana", "buff"),
            hero("Cassidy", "buff"),
            hero("Widow", "nerf"),
            hero("Mercy", "adjustment"),
        ];
        assert_eq!(
            hero_change_counts(&updates),
            [("Buff", 2), ("Nerf", 1), ("Adjustment", 1)]
        );
    }

    #[test]
    fn featured_newest_starts_expanded() {
        assert_eq!(initial_expanded_patches(), vec![0]);
    }

    #[test]
    fn hero_toc_only_at_twelve() {
        assert!(!show_hero_toc(0));
        assert!(!show_hero_toc(11));
        assert!(show_hero_toc(12));
        assert!(show_hero_toc(24));
    }

    #[test]
    fn hero_accordion_is_one_at_a_time() {
        let mut open = HashMap::new();
        open = apply_hero_toggle(&open, 0, 2);
        assert_eq!(open.get(&0).copied(), Some(2));
        open = apply_hero_toggle(&open, 0, 5);
        assert_eq!(open.get(&0).copied(), Some(5));
        open = apply_hero_toggle(&open, 0, 5);
        assert!(!open.contains_key(&0));
        open = apply_hero_toggle(&open, 0, 1);
        open = apply_hero_toggle(&open, 3, 4);
        assert_eq!(open.get(&0).copied(), Some(1));
        assert_eq!(open.get(&3).copied(), Some(4));
    }

    #[test]
    fn change_markers_and_count_labels() {
        assert_eq!(change_marker("buff"), "\u{25B2}");
        assert_eq!(change_marker("nerf"), "\u{25BC}");
        assert_eq!(change_marker("adjustment"), "\u{25C6}");
        assert_eq!(change_marker("bugfix"), "\u{2022}");
        assert_eq!(change_count_label(0), "0 changes");
        assert_eq!(change_count_label(1), "1 change");
        assert_eq!(change_count_label(4), "4 changes");
    }

    #[test]
    fn competitive_chip_compacts_on_mobile_label() {
        assert_eq!(filter_chip_compact_label("Competitive"), "Comp");
        assert_eq!(filter_chip_compact_label("Hero Balance"), "Hero Balance");
        assert_eq!(filter_chip_compact_label("All"), "All");
    }

    #[test]
    fn hero_dom_id_is_stable() {
        assert_eq!(hero_dom_id(0, 3), "patch-0-hero-3");
    }
}
