use std::collections::{HashMap, HashSet};

use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::ui::{
    ChipButton, ChipRow, EmptyCard, LoadError, PageHeader, PageShell, PageWidth, SkeletonBars,
};
use crate::copy::plain_copy;
use crate::hooks::use_api;

// --- Types ---

/// `{ "data": [...] }` envelope: same unwrap as Browse (`StrategyListResponse`).
/// `data` is required so a bare `[]` / `{}` cannot look like a successful empty list.
#[derive(Debug, Clone, Deserialize)]
struct ListResponse {
    #[serde(alias = "patches", deserialize_with = "plain_notes")]
    data: Vec<PatchNote>,
}

/// Local card type. Fields default so a thin API payload still deserializes;
/// unknown keys are ignored. Not a copy of any external patch-notes schema.
#[derive(Debug, Clone, PartialEq, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct PatchChange {
    ability: Option<String>,
    description: String,
    #[serde(default)]
    change_type: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct PatchSection {
    category: String,
    #[serde(default)]
    items: Vec<String>,
}

/// Deserialize the list, then strip long dashes from every free-text field.
/// Version, date, url, hero ids and hero names are identifiers and stay as sent.
fn plain_notes<'de, D>(deserializer: D) -> Result<Vec<PatchNote>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut notes = Vec::<PatchNote>::deserialize(deserializer)?;
    for note in &mut notes {
        plain_note(note);
    }
    Ok(notes)
}

fn plain_note(note: &mut PatchNote) {
    note.title = note.title.take().map(|t| plain_copy(&t));
    note.body = note.body.take().map(|b| plain_copy(&b));
    for hero in &mut note.hero_updates {
        hero.dev_comment = hero.dev_comment.take().map(|c| plain_copy(&c));
        for change in &mut hero.changes {
            change.ability = change.ability.take().map(|a| plain_copy(&a));
            change.description = plain_copy(&change.description);
        }
    }
    for section in &mut note.sections {
        section.category = plain_copy(&section.category);
        for item in &mut section.items {
            *item = plain_copy(item);
        }
    }
}

/// Distinguishes loading vs failed so a missing API cannot look like loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatchFetchState {
    Loading,
    Failed,
    Ready,
}

/// Jump row of hero names only appears once an expanded patch has this many heroes.
const HERO_TOC_THRESHOLD: usize = 12;

/// Input `id` of the toolbar search field. Its name comes from `aria-label`.
const PATCH_SEARCH_ID: &str = "patch-notes-search";

/// The result line. Always rendered; only its text changes.
const PATCH_STATUS_ID: &str = "patch-notes-status";

const NO_MATCH_TEXT: &str = "No patch notes match these filters.";

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

// --- Filters ---

/// The first six are the chips of the frame. Maps, Competitive and General
/// filter on section categories.
const FILTER_OPTIONS: [&str; 9] = [
    "All",
    "Latest",
    "Buffs",
    "Nerfs",
    "Fixes",
    "Adjusts",
    "Maps",
    "Competitive",
    "General",
];

/// A hero update counts as `change_type` when the hero or one of its changes
/// carries it.
fn has_hero_change(patch: &PatchNote, change_type: &str) -> bool {
    patch.hero_updates.iter().any(|h| {
        h.change_type == change_type || h.changes.iter().any(|c| c.change_type == change_type)
    })
}

fn has_section(patch: &PatchNote, words: &[&str]) -> bool {
    patch.sections.iter().any(|s| {
        let cat = s.category.to_lowercase();
        words.iter().any(|w| cat.contains(w))
    })
}

fn patch_matches_filter(patch: &PatchNote, filter: &str) -> bool {
    match filter {
        "All" => true,
        // Position in the catalog, not content: see `note_visible`.
        "Latest" => true,
        "Buffs" => has_hero_change(patch, "buff"),
        "Nerfs" => has_hero_change(patch, "nerf"),
        "Adjusts" => has_hero_change(patch, "adjustment"),
        "Fixes" => has_hero_change(patch, "bugfix") || has_section(patch, &["bug", "fix"]),
        "Maps" => has_section(patch, &["map"]),
        "Competitive" => has_section(patch, &["competitive", "ranked"]),
        "General" => has_section(patch, &["general", "system", "ui"]),
        _ => true,
    }
}

/// Matches version, title, body, hero names, abilities, change text, section
/// names and section items, so the placeholder ("heroes, maps, or text") holds.
fn patch_matches_search(patch: &PatchNote, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let q = query.to_lowercase();
    let hit = |text: &str| text.to_lowercase().contains(&q);
    hit(&patch.version)
        || patch.title.as_deref().is_some_and(hit)
        || patch.body.as_deref().is_some_and(hit)
        || patch.hero_updates.iter().any(|h| {
            hit(&h.hero_name)
                || h.changes
                    .iter()
                    .any(|c| hit(&c.description) || c.ability.as_deref().is_some_and(hit))
        })
        || patch
            .sections
            .iter()
            .any(|s| hit(&s.category) || s.items.iter().any(|item| hit(item)))
}

/// One rule for the list, the count and Expand all. `idx` is the catalog
/// position; the newest note is index 0.
fn note_visible(idx: usize, patch: &PatchNote, query: &str, filter: &str) -> bool {
    (filter != "Latest" || idx == 0)
        && patch_matches_filter(patch, filter)
        && patch_matches_search(patch, query.trim())
}

fn is_filtered(query: &str, filter: &str) -> bool {
    filter != "All" || !query.trim().is_empty()
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

/// Row heading, "2026.11.05 · Season 5 Hotfix". The stamp is the version, or
/// the date when there is no version. A note with no title already names its
/// version, so it gets no stamp.
fn row_heading(patch: &PatchNote) -> String {
    let title = patch_display_title(patch);
    if patch.title.is_none() {
        return title;
    }
    match row_stamp(patch) {
        Some(stamp) => format!("{stamp} \u{b7} {title}"),
        None => title,
    }
}

fn row_stamp(patch: &PatchNote) -> Option<&str> {
    [patch.version.as_str(), patch.date.as_str()]
        .into_iter()
        .find(|s| !s.is_empty())
}

fn total_change_count(patch: &PatchNote) -> usize {
    let hero: usize = patch.hero_updates.iter().map(|h| h.changes.len()).sum();
    let section: usize = patch.sections.iter().map(|s| s.items.len()).sum();
    hero + section
}

/// True when the heading already shows the note's date: the stamp is the date
/// itself, or a version that is that date ("2026.11.05" for 2026-11-05).
fn date_in_heading(patch: &PatchNote) -> bool {
    if patch.title.is_none() || patch.date.is_empty() {
        return false;
    }
    patch.version.is_empty() || patch.version.replace('.', "-") == patch.date
}

/// Row meta, "Latest · 2 heroes · 8 changes", with the date after "Latest"
/// ("Latest · 2026-11-05 · ...") only when the heading does not already show
/// it. Empty parts are skipped.
fn row_meta(patch: &PatchNote, is_latest: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    if is_latest {
        parts.push("Latest".to_string());
    }
    if !patch.date.is_empty() && !date_in_heading(patch) {
        parts.push(patch.date.clone());
    }
    if !patch.hero_updates.is_empty() {
        parts.push(hero_count_label(patch.hero_updates.len()));
    }
    let changes = total_change_count(patch);
    if changes > 0 {
        parts.push(change_count_label(changes));
    }
    parts.join(" \u{b7} ")
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

const LEAD_REST: &str = "Open a row to read the full list. Buff, nerf, fix, and adjust are marked with a word and a shape.";

/// Page lead. Leads with the count once the catalog is loaded and not empty.
fn page_lead(count: Option<(usize, usize, bool)>) -> String {
    match count {
        Some((visible, total, filtered)) => {
            let label = notes_count_label(visible, total, filtered);
            if filtered {
                format!("Showing {label}. {LEAD_REST}")
            } else {
                format!("{label}. {LEAD_REST}")
            }
        }
        None => LEAD_REST.to_string(),
    }
}

/// Text of the status line. "Loading patch notes" during the first load and
/// during a retry; after that, empty until a search or a filter narrows the
/// list.
fn status_text(
    state: PatchFetchState,
    retrying: bool,
    visible: usize,
    total: usize,
    filtered: bool,
) -> String {
    if retrying || state == PatchFetchState::Loading {
        "Loading patch notes".to_string()
    } else if !filtered || total == 0 {
        String::new()
    } else if visible == 0 {
        NO_MATCH_TEXT.to_string()
    } else {
        format!("Showing {}.", notes_count_label(visible, total, true))
    }
}

fn hero_count_label(n: usize) -> String {
    if n == 1 {
        "1 hero".to_string()
    } else {
        format!("{n} heroes")
    }
}

fn show_hero_toc(hero_count: usize) -> bool {
    hero_count >= HERO_TOC_THRESHOLD
}

/// Split heroes so the open accordion sits *outside* the two-column grid and
/// spans the full row width (not one grid cell).
fn hero_open_ranges(
    count: usize,
    open: Option<usize>,
) -> (
    std::ops::Range<usize>,
    Option<usize>,
    std::ops::Range<usize>,
) {
    match open {
        Some(i) if i < count => (0..i, Some(i), i + 1..count),
        _ => (0..count, None, 0..0),
    }
}

/// Lower-case letters and digits of `text`, with every other run as one `-`.
/// "2026.11.05" is "2026-11-05", "Wrecking Ball" is "wrecking-ball".
fn slug(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// `base`, or `base-2`, `base-3` when an earlier row of the same list already
/// took it. Only two rows with the same stem ever get a suffix.
fn unique_key(base: String, used: &mut HashSet<String>) -> String {
    let mut key = base.clone();
    let mut copy = 1;
    while !used.insert(key.clone()) {
        copy += 1;
        key = format!("{base}-{copy}");
    }
    key
}

/// The first non-empty slug of `parts`, else `fallback`.
fn key_stem(parts: &[&str], fallback: &str) -> String {
    parts
        .iter()
        .map(|part| slug(part))
        .find(|part| !part.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

/// One key per note, in catalog order, built from the note itself: its
/// version, else its date, else its title. Element ids and the open/closed
/// state hang off this key, so a refetch that adds a newer note does not move
/// an id or an open row onto a different patch.
fn note_keys(notes: &[PatchNote]) -> Vec<String> {
    let mut used = HashSet::new();
    notes
        .iter()
        .map(|note| {
            let title = note.title.as_deref().unwrap_or_default();
            let stem = key_stem(&[&note.version, &note.date, title], "note");
            unique_key(stem, &mut used)
        })
        .collect()
}

/// One key per hero of a note: the hero id, else the hero name.
fn hero_keys(heroes: &[HeroUpdate]) -> Vec<String> {
    let mut used = HashSet::new();
    heroes
        .iter()
        .map(|hero| {
            unique_key(
                key_stem(&[&hero.hero_id, &hero.hero_name], "hero"),
                &mut used,
            )
        })
        .collect()
}

/// The open rows. Until the reader opens or closes one (`chosen` is `None`)
/// the newest note, the first in the catalog, is open and the rest are closed.
fn expanded_keys(chosen: Option<&[String]>, keys: &[String]) -> Vec<String> {
    match chosen {
        Some(chosen) => chosen.to_vec(),
        None => keys.first().cloned().into_iter().collect(),
    }
}

/// `current` with `key` added, or removed when it was there.
fn toggle_key(current: &[String], key: &str) -> Vec<String> {
    let mut next = current.to_vec();
    match next.iter().position(|k| k == key) {
        Some(pos) => {
            next.remove(pos);
        }
        None => next.push(key.to_string()),
    }
    next
}

fn toggle_open_hero(current: Option<&str>, clicked: &str) -> Option<String> {
    if current == Some(clicked) {
        None
    } else {
        Some(clicked.to_string())
    }
}

/// Expand/Collapse all is **patch**-scoped. Hero accordion state is always
/// cleared so the control cannot reopen every hero (tall-wall regression).
fn apply_patch_bulk_toggle(
    visible: &[String],
    currently_expanded: &[String],
) -> (Vec<String>, HashMap<String, String>) {
    let all_open = !visible.is_empty() && visible.iter().all(|k| currently_expanded.contains(k));
    let patches = if all_open {
        Vec::new()
    } else {
        visible.to_vec()
    };
    (patches, HashMap::new())
}

/// Open hero per patch, both by key.
fn apply_hero_toggle(
    map: &HashMap<String, String>,
    patch_key: &str,
    hero_key: &str,
) -> HashMap<String, String> {
    let mut next = map.clone();
    match toggle_open_hero(next.get(patch_key).map(String::as_str), hero_key) {
        Some(open) => {
            next.insert(patch_key.to_string(), open);
        }
        None => {
            next.remove(patch_key);
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

fn filter_chip_compact_label(label: &str) -> &str {
    match label {
        "Competitive" => "Comp",
        _ => label,
    }
}

fn patch_dom_id(patch_key: &str) -> String {
    format!("patch-{patch_key}")
}

fn hero_dom_id(patch_key: &str, hero_key: &str) -> String {
    format!("patch-{patch_key}-hero-{hero_key}")
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

// --- Verdict ---

/// What a change does. Shown as a word plus a shape, on a soft fill that
/// holds its ink at 4.5:1, so it never reads by colour alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Buff,
    Nerf,
    Adjust,
    Fix,
    Other,
}

impl Verdict {
    const ALL: [Verdict; 5] = [
        Verdict::Buff,
        Verdict::Nerf,
        Verdict::Adjust,
        Verdict::Fix,
        Verdict::Other,
    ];

    fn from_change_type(ct: &str) -> Self {
        match ct {
            "buff" => Verdict::Buff,
            "nerf" => Verdict::Nerf,
            "adjustment" => Verdict::Adjust,
            "bugfix" => Verdict::Fix,
            _ => Verdict::Other,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Verdict::Buff => "Buff",
            Verdict::Nerf => "Nerf",
            Verdict::Adjust => "Adjust",
            Verdict::Fix => "Fix",
            Verdict::Other => "Other",
        }
    }

    /// Modifier class next to `pn-verdict` (badge) or `pn-change-mark` (line).
    fn tone_class(self) -> &'static str {
        match self {
            Verdict::Buff => "is-buff",
            Verdict::Nerf => "is-nerf",
            Verdict::Adjust => "is-adjust",
            Verdict::Fix => "is-fix",
            Verdict::Other => "is-other",
        }
    }

    /// The shape drawn before the word. An unknown type has none.
    fn mark_class(self) -> Option<&'static str> {
        match self {
            Verdict::Buff => Some("pn-mark-up"),
            Verdict::Nerf => Some("pn-mark-down"),
            Verdict::Adjust => Some("pn-mark-diamond"),
            Verdict::Fix => Some("pn-mark-dot"),
            Verdict::Other => None,
        }
    }

    /// `(ink, fill)` tokens of the badge. Only buff and nerf carry a colour;
    /// fix and adjust share the neutral badge, as the frame draws them, and
    /// are told apart by their word and shape.
    fn ink_and_fill(self) -> (&'static str, &'static str) {
        match self {
            Verdict::Buff => ("--win", "--win-soft"),
            Verdict::Nerf => ("--loss", "--loss-soft"),
            Verdict::Adjust | Verdict::Fix | Verdict::Other => ("--text", "--surface"),
        }
    }
}

// --- CSS ---

const PAGE_CSS: &str = r#"
    /* Height of the two sticky layers (site top bar, then these controls). A
       control that takes focus or is scrolled to lands below both. */
    .patch-page {
        --pn-sticky-offset: calc(
            var(--nav-height-used) + var(--hit-target-min) * 2 + var(--space-stack) + var(--space-8)
        );
    }
    /* Sticks under the site top bar, which is sticky on every page. Opaque,
       so rows do not show through. */
    .pn-controls {
        position: sticky;
        top: var(--nav-height-used);
        z-index: 8;
        display: flex;
        flex-direction: column;
        gap: var(--space-stack);
        padding: var(--space-3) 0;
        background: var(--bg);
    }
    .pn-toolbar {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        min-width: 0;
    }
    /* The look is the shared `field-input` and `btn`; these place them. */
    .pn-search {
        flex: 1 1 0;
        min-width: 0;
        height: var(--hit-target-min);
    }
    .pn-expand {
        flex: 0 0 auto;
        white-space: nowrap;
    }
    .pn-list {
        display: flex;
        flex-direction: column;
        gap: var(--space-section);
        min-width: 0;
    }
    .pn-year-block {
        display: flex;
        flex-direction: column;
        gap: var(--space-stack);
        min-width: 0;
    }
    .pn-year {
        margin: 0;
        color: var(--text-3);
        font-family: var(--font-body);
        font-size: var(--text-sm);
    }
    .pn-row {
        min-width: 0;
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
        background: var(--card-bg);
        color: var(--card-fg);
    }
    .pn-row:hover {
        border-color: var(--border-strong);
    }
    .pn-row.is-open {
        border-color: var(--accent-solid);
    }
    .pn-row-title,
    .pn-hero-title,
    .pn-section-title {
        margin: 0;
        color: var(--text);
        font-family: var(--font-head);
        font-weight: var(--font-label-weight);
        line-height: var(--leading-ui);
        letter-spacing: normal;
        text-transform: none;
    }
    .pn-row-title {
        font-size: var(--text-lg);
    }
    .pn-hero-title,
    .pn-section-title {
        font-size: var(--text-md);
    }
    /* The scroll margin keeps a focused header out from behind the sticky
       controls. */
    .pn-row-head,
    .pn-hero-head {
        box-sizing: border-box;
        width: 100%;
        min-height: var(--hit-target-min);
        scroll-margin-top: var(--pn-sticky-offset);
        margin: 0;
        border: 0;
        border-radius: inherit;
        background: transparent;
        color: inherit;
        font: inherit;
        text-align: start;
        cursor: pointer;
    }
    .pn-row-head {
        display: grid;
        grid-template-columns: minmax(0, 1fr) auto;
        align-items: center;
        gap: var(--space-3);
        padding: var(--space-card-pad);
    }
    .pn-row-text {
        display: flex;
        flex-wrap: wrap;
        align-items: baseline;
        justify-content: space-between;
        gap: var(--space-2) var(--space-3);
        min-width: 0;
    }
    .pn-row-name {
        min-width: 0;
        overflow-wrap: anywhere;
    }
    .pn-row-meta {
        color: var(--text-3);
        font-family: var(--font-body);
        font-size: var(--text-sm);
        font-weight: var(--font-body-weight);
    }
    .pn-chevron {
        flex: 0 0 auto;
        color: var(--text-subtle);
        font-size: var(--text-xs);
        line-height: var(--leading-ui);
        transition: transform 0.2s;
    }
    .is-open > .pn-row-title .pn-chevron,
    .is-open > .pn-hero-title .pn-chevron {
        transform: rotate(180deg);
    }
    .pn-row-body {
        display: flex;
        flex-direction: column;
        gap: var(--space-stack);
        min-width: 0;
        padding: 0 var(--space-card-pad) var(--space-card-pad);
    }
    .pn-body-lead {
        display: -webkit-box;
        -webkit-line-clamp: 3;
        -webkit-box-orient: vertical;
        overflow: hidden;
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-md);
        line-height: var(--leading-body);
    }
    /* Heroes: jump row (12 or more heroes), then the list column. The open
       card sits between two grids so it spans the full row width. */
    .pn-hero-block,
    .pn-hero-main {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
        min-width: 0;
    }
    /* The jump row is a shared chip row; a chip scrolled to or focused lands
       below the sticky controls. */
    .pn-toc-chip {
        scroll-margin-top: var(--pn-sticky-offset);
    }
    .pn-heroes {
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: var(--space-3);
    }
    .pn-hero-card {
        min-width: 0;
        border: 1px solid var(--border);
        border-radius: var(--card-radius);
        background: var(--surface);
        scroll-margin-top: var(--pn-sticky-offset);
    }
    .pn-hero-card.is-open {
        grid-column: 1 / -1;
        border-color: var(--accent-solid);
    }
    .pn-hero-head {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2);
        padding: var(--space-2) var(--space-3);
    }
    .pn-hero-name {
        min-width: 0;
        overflow-wrap: anywhere;
    }
    .pn-hero-count {
        margin-inline-start: auto;
        color: var(--text-3);
        font-family: var(--font-body);
        font-size: var(--text-xs);
        font-weight: var(--font-body-weight);
        white-space: nowrap;
    }
    .pn-hero-body {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
        padding: 0 var(--space-3) var(--space-3);
    }
    .pn-verdict {
        box-sizing: border-box;
        display: inline-flex;
        align-items: center;
        gap: var(--space-1);
        min-height: var(--space-5);
        padding: 0 var(--badge-pad-x);
        border: 1px solid transparent;
        border-radius: var(--badge-radius);
        font-family: var(--badge-font);
        font-size: var(--badge-font-size);
        font-weight: var(--badge-font-weight);
        letter-spacing: var(--font-label-tracking);
        line-height: var(--leading-ui);
        text-transform: var(--badge-font-transform);
        white-space: nowrap;
    }
    .pn-verdict.is-buff {
        color: var(--win);
        background: var(--win-soft);
    }
    .pn-verdict.is-nerf {
        color: var(--loss);
        background: var(--loss-soft);
    }
    .pn-verdict.is-fix,
    .pn-verdict.is-adjust,
    .pn-verdict.is-other {
        color: var(--text);
        background: var(--surface);
        border-color: var(--border-strong);
    }
    .pn-mark {
        display: inline-block;
        flex: 0 0 auto;
    }
    .pn-mark-up,
    .pn-mark-down {
        width: 0;
        height: 0;
        border-left: var(--space-1) solid transparent;
        border-right: var(--space-1) solid transparent;
    }
    .pn-mark-up {
        border-bottom: var(--space-2) solid currentColor;
    }
    .pn-mark-down {
        border-top: var(--space-2) solid currentColor;
    }
    .pn-mark-diamond,
    .pn-mark-dot {
        width: calc(var(--space-2) * 0.75);
        height: calc(var(--space-2) * 0.75);
        background: currentColor;
    }
    .pn-mark-diamond {
        transform: rotate(45deg);
    }
    .pn-mark-dot {
        border-radius: var(--radius-pill);
    }
    .pn-dev-comment {
        margin: 0;
        padding-inline-start: var(--space-3);
        border-inline-start: 1px solid var(--border-strong);
        color: var(--text-3);
        font-size: var(--text-sm);
        font-style: italic;
        line-height: var(--leading-body);
    }
    .pn-change-list {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
        margin: 0;
        padding: 0;
        list-style: none;
    }
    .pn-change {
        display: flex;
        align-items: baseline;
        gap: var(--space-2);
        color: var(--text);
        font-size: var(--text-md);
        line-height: var(--leading-body);
    }
    /* The bullet of a plain line. A line whose type differs from its hero's
       carries the verdict badge (word and shape) in its place. */
    .pn-change-mark {
        flex: 0 0 auto;
        color: var(--text-subtle);
        font-size: var(--text-xs);
        font-weight: var(--font-label-weight);
    }
    .pn-change > .pn-verdict {
        flex: 0 0 auto;
        align-self: flex-start;
    }
    .pn-change-copy {
        min-width: 0;
        overflow-wrap: anywhere;
    }
    .pn-change-ability {
        font-weight: var(--font-label-weight);
    }
    .pn-section {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
        min-width: 0;
    }
    .pn-section-title {
        padding-inline-start: var(--space-3);
        border-inline-start: var(--space-1) solid var(--accent-solid);
    }
    .pn-section-items {
        margin: 0;
        padding-inline-start: var(--space-5);
        color: var(--text);
        font-size: var(--text-md);
        line-height: var(--leading-body);
        overflow-wrap: anywhere;
    }
    .pn-external-link {
        align-self: flex-start;
        display: inline-flex;
        align-items: center;
        min-height: var(--hit-target-min);
        color: var(--accent-text);
        font-size: var(--text-sm);
        text-decoration: underline;
        text-underline-offset: var(--space-1);
        scroll-margin-top: var(--pn-sticky-offset);
    }
    .pn-external-text {
        color: var(--text-3);
        font-size: var(--text-sm);
        word-break: break-all;
    }
    @media (max-width: 820px) {
        .pn-heroes {
            grid-template-columns: minmax(0, 1fr);
        }
    }
"#;

// --- Component ---

/// Shared Patch Notes UI: used by `/patch-notes` (PublicLayout) and
/// `/strategy/patch-notes` (StrategyLayout). Same fetch + unwrap.
#[component]
pub fn PatchNotesPage() -> Element {
    let patches = use_api::<ListResponse>("/api/strategy/patch-notes");
    let mut refresh = patches.refresh;

    let (state, notes) = {
        let data = patches.data.read();
        let err = patches.error.read();
        let state = classify_patch_fetch(data.as_ref().map(|d| d.as_ref()), err.as_deref());
        let notes = data
            .as_ref()
            .and_then(|d| d.as_ref())
            .map(|resp| resp.data.clone())
            .unwrap_or_default();
        (state, notes)
    };
    // `use_api` keeps the failed slot while a retry is in flight.
    let retrying =
        state == PatchFetchState::Failed && patches.data.state()() == UseResourceState::Pending;

    rsx! {
        PatchNotesView { state, notes, retrying, on_retry: move |_| refresh += 1 }
    }
}

/// The page for one fetch state. Owns search, filter and open/closed state.
/// `initial_query` and `initial_filter` seed that state on first render.
#[component]
fn PatchNotesView(
    state: PatchFetchState,
    notes: Vec<PatchNote>,
    on_retry: EventHandler<()>,
    /// A Retry is running.
    #[props(default = false)]
    retrying: bool,
    #[props(default)] initial_query: String,
    #[props(default = "All".to_string())] initial_filter: String,
) -> Element {
    let mut search_query = use_signal(|| initial_query.clone());
    let active_filter = use_signal(|| initial_filter.clone());
    // Open rows and the open hero per row, by note key. `None` until the
    // reader opens or closes a row: the newest note is open by default.
    let mut expanded: Signal<Option<Vec<String>>> = use_signal(|| None);
    let mut open_hero: Signal<HashMap<String, String>> = use_signal(HashMap::new);

    let query = search_query();
    let filter = active_filter();
    let ready = matches!(state, PatchFetchState::Ready);
    let total = if ready { notes.len() } else { 0 };
    let filtered = is_filtered(&query, &filter);
    let keys = note_keys(&notes);
    let visible: Vec<(usize, &PatchNote)> = if ready {
        notes
            .iter()
            .enumerate()
            .filter(|(idx, patch)| note_visible(*idx, patch, &query, &filter))
            .collect()
    } else {
        Vec::new()
    };
    let show_toolbar = ready && total > 0;
    let lead = page_lead(show_toolbar.then_some((visible.len(), total, filtered)));
    let status = status_text(state, retrying, visible.len(), total, filtered);
    let visible_keys: Vec<String> = visible.iter().map(|(idx, _)| keys[*idx].clone()).collect();
    let open_rows = expanded_keys(expanded.read().as_deref(), &keys);
    let all_open = !visible_keys.is_empty() && visible_keys.iter().all(|k| open_rows.contains(k));
    let bulk_label = if all_open {
        "Collapse all"
    } else {
        "Expand all"
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-strategy-patch_notes-page-css", css: PAGE_CSS }

        PageShell { width: PageWidth::Narrow, class: "patch-page",
            PageHeader { title: "Patch notes", lead }

            if show_toolbar {
                div { class: "pn-controls",
                    div { class: "pn-toolbar",
                        input {
                            id: PATCH_SEARCH_ID,
                            class: "field-input pn-search",
                            r#type: "search",
                            aria_label: "Search patch notes",
                            placeholder: "Search heroes, maps, or text",
                            value: "{search_query}",
                            oninput: move |evt| search_query.set(evt.value()),
                        }
                        if visible.len() > 1 {
                            button {
                                class: "btn is-ghost pn-expand",
                                r#type: "button",
                                title: "Expand or collapse patches. Heroes stay collapsed.",
                                onclick: {
                                    let open_rows = open_rows.clone();
                                    move |_| {
                                        let (next_patches, next_heroes) =
                                            apply_patch_bulk_toggle(&visible_keys, &open_rows);
                                        expanded.set(Some(next_patches));
                                        open_hero.set(next_heroes);
                                    }
                                },
                                "{bulk_label}"
                            }
                        }
                    }
                    ChipRow { label: "Filter patch notes", scroll: true,
                        for label in FILTER_OPTIONS {
                            {render_filter_chip(label, &filter, active_filter)}
                        }
                    }
                }
            }

            p { id: PATCH_STATUS_ID, class: "visually-hidden", role: "status", "{status}" }

            match state {
                // The words are the status line above; the bars are decoration.
                PatchFetchState::Loading => rsx! { SkeletonBars { rows: 5 } },
                PatchFetchState::Failed => rsx! {
                    LoadError {
                        message: "Couldn't load patch notes. Check your connection and try again."
                            .to_string(),
                        on_retry: move |_| on_retry.call(()),
                        retrying,
                    }
                },
                PatchFetchState::Ready if notes.is_empty() => rsx! {
                    EmptyCard {
                        title: "No patch notes yet",
                        message: "When the catalog is published they will show up here.",
                    }
                },
                PatchFetchState::Ready if visible.is_empty() => rsx! {
                    EmptyCard {
                        title: "No matching patch notes",
                        message: "Try clearing search or switching the filter.",
                    }
                },
                PatchFetchState::Ready => rsx! {
                    div { class: "pn-list",
                        for (year, rows) in group_by_year(&visible) {
                            div { class: "pn-year-block",
                                p { class: "pn-year", "{year}" }
                                for (idx, patch) in rows.iter() {
                                    {render_patch_row(
                                        PatchRow {
                                            key: keys[*idx].clone(),
                                            is_latest: *idx == 0,
                                            open_rows: open_rows.clone(),
                                        },
                                        patch,
                                        expanded,
                                        open_hero,
                                    )}
                                }
                            }
                        }
                    }
                },
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

/// A shared chip. "Competitive" shows a shorter word at phone width; its
/// accessible name stays the full word.
fn render_filter_chip(label: &'static str, current: &str, mut signal: Signal<String>) -> Element {
    let compact = filter_chip_compact_label(label);
    let short_label = (compact != label).then(|| compact.to_string());

    rsx! {
        ChipButton {
            class: "chip",
            label,
            short_label,
            pressed: label == current,
            onclick: move |_| signal.set(label.to_string()),
        }
    }
}

/// What a row needs besides its note: its key, whether it is the newest note,
/// and the rows open right now.
struct PatchRow {
    key: String,
    is_latest: bool,
    open_rows: Vec<String>,
}

fn render_patch_row(
    row: PatchRow,
    patch: &PatchNote,
    mut expanded: Signal<Option<Vec<String>>>,
    open_hero: Signal<HashMap<String, String>>,
) -> Element {
    let PatchRow {
        key,
        is_latest,
        open_rows,
    } = row;
    let is_expanded = open_rows.contains(&key);
    let heading = row_heading(patch);
    let meta = row_meta(patch, is_latest);
    let row_id = patch_dom_id(&key);
    let body_id = format!("{row_id}-body");
    let url = patch.url.clone();
    let safe_href = crate::util::http_href(&url).map(str::to_string);
    let lead = patch.body.clone().filter(|s| !s.is_empty());
    let keys = hero_keys(&patch.hero_updates);
    let open_key = open_hero.read().get(&key).cloned();
    let open_idx = open_key
        .as_deref()
        .and_then(|open| keys.iter().position(|k| k == open));
    let toc = show_hero_toc(patch.hero_updates.len());
    let heroes = HeroList {
        patch_key: &key,
        keys: &keys,
        heroes: &patch.hero_updates,
        open_idx,
        open_hero,
    };

    rsx! {
        div {
            class: if is_expanded { "pn-row is-open" } else { "pn-row" },
            id: "{row_id}",
            h2 { class: "pn-row-title",
                button {
                    class: "pn-row-head",
                    r#type: "button",
                    aria_expanded: if is_expanded { "true" } else { "false" },
                    aria_controls: is_expanded.then(|| body_id.clone()),
                    onclick: {
                        let key = key.clone();
                        move |_| expanded.set(Some(toggle_key(&open_rows, &key)))
                    },
                    span { class: "pn-row-text",
                        span { class: "pn-row-name", "{heading}" }
                        " "
                        span { class: "pn-row-meta", "{meta}" }
                    }
                    span { class: "pn-chevron", aria_hidden: "true", "\u{25bc}" }
                }
            }

            if is_expanded {
                div { class: "pn-row-body", id: "{body_id}",
                    if let Some(lead) = lead {
                        p { class: "pn-body-lead", "{lead}" }
                    }

                    if !patch.hero_updates.is_empty() {
                        // Named after the row, so two open patches do not
                        // expose two groups with the same name.
                        div {
                            class: "pn-hero-block",
                            role: "group",
                            aria_label: "Hero changes in {heading}",
                            if toc {
                                ChipRow { label: "Heroes in {heading}", scroll: true,
                                    for hi in 0..patch.hero_updates.len() {
                                        {render_toc_item(&heroes, hi)}
                                    }
                                }
                            }
                            div { class: "pn-hero-main", {render_hero_column(&heroes)} }
                        }
                    }

                    for section in patch.sections.iter() {
                        {render_section(section)}
                    }

                    if let Some(href) = safe_href {
                        a {
                            class: "pn-external-link",
                            href: "{href}",
                            target: "_blank",
                            rel: "noopener noreferrer",
                            aria_label: "View official patch notes for {heading} (opens in a new tab)",
                            "View official patch notes "
                            span { aria_hidden: "true", "\u{2192}" }
                        }
                    } else if !url.trim().is_empty() {
                        span { class: "pn-external-text", "{url}" }
                    }
                }
            }
        }
    }
}

/// The heroes of one open row, with their keys and the open one.
struct HeroList<'a> {
    patch_key: &'a str,
    keys: &'a [String],
    heroes: &'a [HeroUpdate],
    open_idx: Option<usize>,
    open_hero: Signal<HashMap<String, String>>,
}

impl HeroList<'_> {
    /// Click handler of hero `hi`: toggle it, and bring it into view when it
    /// opens.
    fn toggle(&self, hi: usize) -> impl FnMut() + 'static {
        let patch_key = self.patch_key.to_string();
        let hero_key = self.keys[hi].clone();
        let mut open_hero = self.open_hero;
        move || {
            let current = open_hero.peek().clone();
            let opening = current.get(&patch_key) != Some(&hero_key);
            open_hero.set(apply_hero_toggle(&current, &patch_key, &hero_key));
            if opening {
                scroll_to_hero_card(&hero_dom_id(&patch_key, &hero_key));
            }
        }
    }
}

/// One chip of the jump row (12 or more heroes): opens that hero's card.
fn render_toc_item(list: &HeroList<'_>, hi: usize) -> Element {
    let mut toggle = list.toggle(hi);

    rsx! {
        ChipButton {
            class: "chip pn-toc-chip",
            label: list.heroes[hi].hero_name.clone(),
            pressed: list.open_idx == Some(hi),
            onclick: move |_| toggle(),
        }
    }
}

fn render_hero_column(list: &HeroList<'_>) -> Element {
    let (before, open, after) = hero_open_ranges(list.heroes.len(), list.open_idx);

    rsx! {
        if !before.is_empty() {
            div { class: "pn-heroes",
                for hi in before {
                    {render_hero_update(list, hi)}
                }
            }
        }
        if let Some(hi) = open {
            {render_hero_update(list, hi)}
        }
        if !after.is_empty() {
            div { class: "pn-heroes",
                for hi in after {
                    {render_hero_update(list, hi)}
                }
            }
        }
    }
}

/// The badge: a shape (hidden from assistive tech) plus the word.
fn render_verdict(verdict: Verdict) -> Element {
    let tone = verdict.tone_class();
    let label = verdict.label();

    rsx! {
        span { class: "pn-verdict {tone}",
            if let Some(mark) = verdict.mark_class() {
                span { class: "pn-mark {mark}", aria_hidden: "true" }
            }
            "{label}"
        }
    }
}

fn render_hero_update(list: &HeroList<'_>, hi: usize) -> Element {
    let hu = &list.heroes[hi];
    let is_open = list.open_idx == Some(hi);
    let verdict = Verdict::from_change_type(&hu.change_type);
    let count_label = change_count_label(hu.changes.len());
    let dom_id = hero_dom_id(list.patch_key, &list.keys[hi]);
    let body_id = format!("{dom_id}-body");
    let hero_name = hu.hero_name.clone();
    let mut toggle = list.toggle(hi);

    rsx! {
        div {
            class: if is_open { "pn-hero-card is-open" } else { "pn-hero-card" },
            id: "{dom_id}",
            h3 { class: "pn-hero-title",
                button {
                    class: "pn-hero-head",
                    r#type: "button",
                    aria_expanded: if is_open { "true" } else { "false" },
                    aria_controls: is_open.then(|| body_id.clone()),
                    onclick: move |_| toggle(),
                    span { class: "pn-hero-name", "{hero_name}" }
                    " "
                    {render_verdict(verdict)}
                    " "
                    span { class: "pn-hero-count", "{count_label}" }
                    span { class: "pn-chevron", aria_hidden: "true", "\u{25bc}" }
                }
            }

            if is_open {
                div { class: "pn-hero-body", id: "{body_id}",
                    if let Some(comment) = &hu.dev_comment {
                        p { class: "pn-dev-comment", "\"{comment}\"" }
                    }

                    if !hu.changes.is_empty() {
                        ul { class: "pn-change-list",
                            for change in hu.changes.iter() {
                                {render_change_item(change, verdict)}
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The verdict a change line shows, if any. The hero's badge already says
/// what its changes are, so a line only carries a badge when its own type is
/// a different one. Everyone gets the same words: nothing is left to a glyph
/// or to hidden text.
fn line_verdict(change_type: &str, hero: Verdict) -> Option<Verdict> {
    let verdict = Verdict::from_change_type(change_type);
    (verdict != Verdict::Other && verdict != hero).then_some(verdict)
}

/// One change line: the verdict badge when the line differs from its hero,
/// else a plain bullet.
fn render_change_item(change: &PatchChange, hero: Verdict) -> Element {
    let verdict = line_verdict(&change.change_type, hero);

    rsx! {
        li { class: "pn-change",
            if let Some(verdict) = verdict {
                {render_verdict(verdict)}
            } else {
                span { class: "pn-change-mark", aria_hidden: "true", "\u{2022}" }
            }
            span { class: "pn-change-copy",
                if let Some(ability) = &change.ability {
                    span { class: "pn-change-ability", "{ability}: " }
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
        section { class: "pn-section",
            h3 { class: "pn-section-title", "{section.category}" }
            ul { class: "pn-section-items",
                for item in section.items.iter() {
                    li { "{item}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{
        assert_ink_clears_aa, assert_sizes_from_tokens, css_rule, has_decl, render,
    };

    fn change(description: &str, change_type: &str) -> PatchChange {
        PatchChange {
            ability: None,
            description: description.into(),
            change_type: change_type.into(),
        }
    }

    /// Two notes in two years. The newest has a buffed and a fixed hero plus
    /// one section; the older one has one nerfed hero.
    fn fixture() -> Vec<PatchNote> {
        let mut ball = hero("Wrecking Ball", "buff");
        ball.changes = vec![
            change("Grappling Claw cooldown reduced.", "buff"),
            change("Piledriver knockback increased.", "buff"),
        ];
        let dva = hero("D.Va", "bugfix");
        let mut mercy = hero("Mercy", "nerf");
        mercy.changes = vec![change("Healing reduced.", "nerf")];
        vec![
            note(
                "9.1",
                "2026-11-05",
                Some("Season 9 Hotfix"),
                vec![ball, dva],
                vec![PatchSection {
                    category: "General".into(),
                    items: vec!["Custom games keep the last lobby settings.".into()],
                }],
            ),
            note(
                "8.0",
                "2025-10-22",
                Some("Season 8 Launch"),
                vec![mercy],
                vec![],
            ),
        ]
    }

    fn view(state: PatchFetchState, notes: Vec<PatchNote>, query: &str, filter: &str) -> Element {
        rsx! {
            PatchNotesView {
                state,
                notes,
                on_retry: move |_| {},
                initial_query: query.to_string(),
                initial_filter: filter.to_string(),
            }
        }
    }

    fn ready_app() -> Element {
        view(PatchFetchState::Ready, fixture(), "", "All")
    }

    fn search_hit_app() -> Element {
        view(PatchFetchState::Ready, fixture(), "mercy", "All")
    }

    fn no_match_app() -> Element {
        view(PatchFetchState::Ready, fixture(), "junkrat", "All")
    }

    fn latest_app() -> Element {
        view(PatchFetchState::Ready, fixture(), "", "Latest")
    }

    fn loading_app() -> Element {
        view(PatchFetchState::Loading, vec![], "", "All")
    }

    fn failed_app() -> Element {
        view(PatchFetchState::Failed, vec![], "", "All")
    }

    fn retrying_app() -> Element {
        rsx! {
            PatchNotesView {
                state: PatchFetchState::Failed,
                notes: vec![],
                retrying: true,
                on_retry: move |_| {},
            }
        }
    }

    fn two_open_app() -> Element {
        let mut notes = fixture();
        notes[0].url = "https://example.com/patch-9-1".into();
        // One row cannot be the only one open, so render the newest alone.
        view(PatchFetchState::Ready, notes, "", "Latest")
    }

    fn empty_app() -> Element {
        view(PatchFetchState::Ready, vec![], "", "All")
    }

    fn toc_app() -> Element {
        let heroes = (1..=12)
            .map(|n| hero(&format!("Hero{n:02}"), "buff"))
            .collect();
        let notes = vec![note("1.0", "2026-01-01", Some("Big patch"), heroes, vec![])];
        view(PatchFetchState::Ready, notes, "", "All")
    }

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
    fn filter_buffs_requires_a_buffed_hero() {
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
        assert!(patch_matches_filter(&with_heroes, "Buffs"));
        assert!(!patch_matches_filter(&with_heroes, "Nerfs"));
        assert!(!patch_matches_filter(&without, "Buffs"));
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
    fn row_heading_stamps_the_title_with_version_or_date() {
        let both = note("2.15", "2026-09-01", Some("Mid-Season"), vec![], vec![]);
        assert_eq!(row_heading(&both), "2.15 \u{b7} Mid-Season");
        let dated = note("", "2026-09-01", Some("Hotfix"), vec![], vec![]);
        assert_eq!(row_heading(&dated), "2026-09-01 \u{b7} Hotfix");
        let bare = note("", "", Some("Hotfix"), vec![], vec![]);
        assert_eq!(row_heading(&bare), "Hotfix");
        let untitled = note("2.15", "2026-09-01", None, vec![], vec![]);
        assert_eq!(row_heading(&untitled), "Patch 2.15");
    }

    #[test]
    fn row_meta_counts_heroes_and_changes() {
        let mut ana = hero("Ana", "buff");
        ana.changes = vec![change("Damage increased", "buff")];
        let patch = note(
            "1",
            "2026-01-01",
            Some("Balance"),
            vec![ana, hero("Mercy", "nerf")],
            vec![PatchSection {
                category: "Bug Fixes".into(),
                items: vec!["fix one".into(), "fix two".into()],
            }],
        );
        assert_eq!(
            row_meta(&patch, true),
            "Latest \u{b7} 2026-01-01 \u{b7} 2 heroes \u{b7} 3 changes"
        );
        assert_eq!(
            row_meta(&patch, false),
            "2026-01-01 \u{b7} 2 heroes \u{b7} 3 changes"
        );
        // The heading already shows the date when there is no version.
        let dated = note("", "2026-01-01", Some("Hotfix"), vec![], vec![]);
        assert_eq!(row_meta(&dated, false), "");
        // A version that is the date shows it once too, as in the frame:
        // "2026.11.05 · Season 5 Hotfix" over "Latest · 2 heroes · 8 changes".
        let stamped = note(
            "2026.11.05",
            "2026-11-05",
            Some("Season 5 Hotfix"),
            vec![hero("Ana", "buff")],
            vec![],
        );
        assert_eq!(row_heading(&stamped), "2026.11.05 \u{b7} Season 5 Hotfix");
        assert_eq!(row_meta(&stamped, true), "Latest \u{b7} 1 hero");
        // An untitled note names only its version, so its date stays.
        let untitled = note("2026.11.05", "2026-11-05", None, vec![], vec![]);
        assert_eq!(row_meta(&untitled, false), "2026-11-05");
    }

    #[test]
    fn latest_filter_keeps_only_the_newest_note() {
        let a = note("2", "2026-09-01", Some("Sep"), vec![], vec![]);
        let b = note("1", "2026-01-01", Some("Jan"), vec![], vec![]);
        assert!(note_visible(0, &a, "", "Latest"));
        assert!(!note_visible(1, &b, "", "Latest"));
        assert!(!note_visible(0, &a, "jan", "Latest"));
        assert!(note_visible(1, &b, "", "All"));
        // Blank search is no search.
        assert!(note_visible(1, &b, "   ", "All"));
        assert!(!is_filtered("   ", "All"));
        assert!(is_filtered("", "Latest"));
        assert!(is_filtered("ana", "All"));
    }

    #[test]
    fn notes_count_label_reports_filter_state() {
        assert_eq!(notes_count_label(35, 35, false), "35 notes");
        assert_eq!(notes_count_label(1, 1, false), "1 note");
        assert_eq!(notes_count_label(4, 35, true), "4 of 35");
        assert_eq!(notes_count_label(1, 35, true), "1 of 35");
    }

    #[test]
    fn verdict_filters_read_hero_and_change_types() {
        let mut mixed = hero("Ana", "adjustment");
        mixed.changes = vec![change("Cooldown up", "nerf")];
        let patch = note(
            "1",
            "2026-01-01",
            None,
            vec![mixed, hero("D.Va", "bugfix")],
            vec![],
        );
        assert!(patch_matches_filter(&patch, "Adjusts"));
        assert!(patch_matches_filter(&patch, "Nerfs"));
        assert!(patch_matches_filter(&patch, "Fixes"));
        assert!(!patch_matches_filter(&patch, "Buffs"));
        assert!(!patch_matches_filter(&patch, "Maps"));
        let sections = note(
            "1",
            "2026-01-01",
            None,
            vec![],
            vec![PatchSection {
                category: "Map Updates".into(),
                items: vec!["Route 66 lighting".into()],
            }],
        );
        assert!(patch_matches_filter(&sections, "Maps"));
        assert!(!patch_matches_filter(&sections, "Fixes"));
        assert!(patch_matches_search(&sections, "route 66"));
        assert!(patch_matches_search(&sections, "map updates"));
        assert!(patch_matches_search(&patch, "cooldown"));
    }

    fn keys_of(list: &[&str]) -> Vec<String> {
        list.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn featured_newest_starts_expanded() {
        let keys = keys_of(&["9-1", "8-0"]);
        // Untouched: the newest note (first in the catalog) is open.
        assert_eq!(expanded_keys(None, &keys), ["9-1"]);
        assert!(expanded_keys(None, &[]).is_empty());
        // Once the reader has chosen, their choice stands, even "none open".
        assert!(expanded_keys(Some(&[]), &keys).is_empty());
        assert_eq!(expanded_keys(Some(&keys_of(&["8-0"])), &keys), ["8-0"]);
        assert_eq!(toggle_key(&keys_of(&["9-1"]), "8-0"), ["9-1", "8-0"]);
        assert_eq!(toggle_key(&keys_of(&["9-1", "8-0"]), "9-1"), ["8-0"]);
    }

    #[test]
    fn keys_come_from_the_note_and_survive_a_newer_note() {
        assert_eq!(slug("2026.11.05"), "2026-11-05");
        assert_eq!(slug("Wrecking Ball"), "wrecking-ball");
        assert_eq!(slug("  D.Va!  "), "d-va");
        assert_eq!(slug("..."), "");

        let old = fixture();
        assert_eq!(note_keys(&old), ["9-1", "8-0"]);
        // A refetch that prepends a newer note leaves every older key alone,
        // so ids and open rows stay on the same patch.
        let mut newer = vec![note("9.2", "2026-11-12", Some("Hotfix"), vec![], vec![])];
        newer.extend(old);
        assert_eq!(note_keys(&newer), ["9-2", "9-1", "8-0"]);

        // Version, else date, else title, else a plain word.
        let thin = vec![
            note("", "2026-08-12", Some("Hotfix"), vec![], vec![]),
            note("", "", Some("Client update"), vec![], vec![]),
            note("", "", None, vec![], vec![]),
        ];
        assert_eq!(note_keys(&thin), ["2026-08-12", "client-update", "note"]);

        // Two rows with one stem still get two ids.
        let twins = vec![
            note("9.1", "2026-11-05", None, vec![], vec![]),
            note("9.1", "2026-11-06", None, vec![], vec![]),
        ];
        assert_eq!(note_keys(&twins), ["9-1", "9-1-2"]);

        let heroes = vec![hero("Wrecking Ball", "buff"), hero("D.Va", "bugfix")];
        assert_eq!(hero_keys(&heroes), ["wrecking-ball", "d-va"]);
        let mut unnamed_id = hero("Ana", "buff");
        unnamed_id.hero_id = String::new();
        assert_eq!(hero_keys(&[unnamed_id]), ["ana"]);
    }

    #[test]
    fn hero_toc_only_at_twelve() {
        assert!(!show_hero_toc(0));
        assert!(!show_hero_toc(11));
        assert!(show_hero_toc(12));
        assert!(show_hero_toc(24));
    }

    #[test]
    fn open_hero_is_split_out_of_collapsed_grid() {
        let (before, open, after) = hero_open_ranges(24, None);
        assert_eq!(before, 0..24);
        assert_eq!(open, None);
        assert!(after.is_empty());

        let (before, open, after) = hero_open_ranges(24, Some(0));
        assert!(before.is_empty());
        assert_eq!(open, Some(0));
        assert_eq!(after, 1..24);

        let (before, open, after) = hero_open_ranges(24, Some(5));
        assert_eq!(before, 0..5);
        assert_eq!(open, Some(5));
        assert_eq!(after, 6..24);

        let (before, open, after) = hero_open_ranges(24, Some(23));
        assert_eq!(before, 0..23);
        assert_eq!(open, Some(23));
        assert!(after.is_empty());

        let (before, open, after) = hero_open_ranges(24, Some(99));
        assert_eq!(before, 0..24);
        assert_eq!(open, None);
        assert!(after.is_empty());
    }

    #[test]
    fn search_field_is_named_by_aria_label() {
        assert_eq!(PATCH_SEARCH_ID, "patch-notes-search");
        let html = render(ready_app);
        assert!(
            html.contains("id=\"patch-notes-search\" class=\"field-input pn-search\" type=\"search\" aria-label=\"Search patch notes\""),
            "{html}"
        );
        assert!(
            html.contains("placeholder=\"Search heroes, maps, or text\""),
            "{html}"
        );
        assert!(!html.contains("<label"), "{html}");
    }

    #[test]
    fn open_hero_css_spans_the_list_column() {
        assert!(
            has_decl(
                &css_rule(PAGE_CSS, ".pn-hero-card.is-open {"),
                "grid-column: 1 / -1"
            ),
            "an open card left in a grid still spans the full row"
        );
        assert!(
            has_decl(
                &css_rule(PAGE_CSS, ".pn-heroes {"),
                "grid-template-columns: repeat(2, minmax(0, 1fr))"
            ),
            "closed hero cards sit two up on desktop"
        );
        let phone = PAGE_CSS
            .split("@media (max-width: 820px)")
            .nth(1)
            .expect("phone block");
        assert!(
            has_decl(
                &css_rule(phone, ".pn-heroes {"),
                "grid-template-columns: minmax(0, 1fr)"
            ),
            "hero cards stack to one column on phone"
        );
        // The jump row sits above the list column, so the open card never
        // becomes a narrow side track. It is a shared chip row, named after
        // its patch.
        let html = render(toc_app);
        let toc = html
            .find(
                "<div class=\"chip-row is-scroll\" role=\"group\" aria-label=\"Heroes in 1.0 \u{b7} Big patch\">",
            )
            .expect("jump row");
        let main = html.find("class=\"pn-hero-main\"").expect("list column");
        assert!(toc < main, "{html}");
        assert_eq!(
            html.matches("class=\"chip pn-toc-chip\"").count(),
            12,
            "{html}"
        );
        assert!(
            html.contains(
                "<button type=\"button\" aria-pressed=\"false\" class=\"chip pn-toc-chip\">Hero01</button>"
            ),
            "{html}"
        );
        // No hand-made chip is left for it.
        assert!(!PAGE_CSS.contains("pn-hero-toc"), "use the shared chip");
    }

    #[test]
    fn expand_all_opens_patches_not_heroes() {
        let visible = keys_of(&["9-1", "8-0", "7-2"]);
        let leftover = apply_hero_toggle(&HashMap::new(), "9-1", "ana");
        assert_eq!(leftover.get("9-1").map(String::as_str), Some("ana"));

        let (opened, heroes) = apply_patch_bulk_toggle(&visible, &keys_of(&["9-1"]));
        assert_eq!(opened, visible);
        assert!(
            heroes.is_empty(),
            "Expand all must not open hero accordions (tall-wall regression)"
        );

        let (collapsed, heroes_after) = apply_patch_bulk_toggle(&visible, &opened);
        assert!(collapsed.is_empty());
        assert!(heroes_after.is_empty());
    }

    #[test]
    fn hero_accordion_is_one_at_a_time() {
        let at = |map: &HashMap<String, String>, patch: &str| map.get(patch).cloned();
        let mut open = HashMap::new();
        open = apply_hero_toggle(&open, "9-1", "ana");
        assert_eq!(at(&open, "9-1").as_deref(), Some("ana"));
        open = apply_hero_toggle(&open, "9-1", "mercy");
        assert_eq!(at(&open, "9-1").as_deref(), Some("mercy"));
        open = apply_hero_toggle(&open, "9-1", "mercy");
        assert!(!open.contains_key("9-1"));
        open = apply_hero_toggle(&open, "9-1", "d-va");
        open = apply_hero_toggle(&open, "8-0", "lucio");
        assert_eq!(at(&open, "9-1").as_deref(), Some("d-va"));
        assert_eq!(at(&open, "8-0").as_deref(), Some("lucio"));
    }

    /// A change line says its type in visible words whenever it is not the
    /// hero's own. A fix line is not left looking like an untyped one, and
    /// nothing is said only to assistive tech.
    #[test]
    fn change_lines_show_a_verdict_only_when_it_differs_from_the_hero() {
        assert_eq!(line_verdict("buff", Verdict::Buff), None);
        assert_eq!(line_verdict("nerf", Verdict::Buff), Some(Verdict::Nerf));
        assert_eq!(line_verdict("bugfix", Verdict::Buff), Some(Verdict::Fix));
        assert_eq!(
            line_verdict("adjustment", Verdict::Other),
            Some(Verdict::Adjust)
        );
        assert_eq!(line_verdict("", Verdict::Buff), None);

        fn lines() -> Element {
            let same = PatchChange {
                ability: Some("Biotic Rifle".into()),
                description: "Damage increased.".into(),
                change_type: "buff".into(),
            };
            let fix = PatchChange {
                ability: None,
                description: "No longer clips the payload.".into(),
                change_type: "bugfix".into(),
            };
            rsx! {
                ul {
                    {render_change_item(&same, Verdict::Buff)}
                    {render_change_item(&fix, Verdict::Buff)}
                }
            }
        }
        let html = render(lines);
        assert!(
            html.contains(
                "<li class=\"pn-change\"><span class=\"pn-change-mark\" aria-hidden=\"true\">\u{2022}</span><span class=\"pn-change-copy\"><span class=\"pn-change-ability\">Biotic Rifle: </span>Damage increased.</span></li>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<li class=\"pn-change\"><span class=\"pn-verdict is-fix\"><span class=\"pn-mark pn-mark-dot\" aria-hidden=\"true\"></span>Fix</span><span class=\"pn-change-copy\">No longer clips the payload.</span></li>"
            ),
            "{html}"
        );
        assert!(!html.contains("visually-hidden"), "{html}");
        assert!(
            !PAGE_CSS.contains(".pn-change-mark.is-"),
            "no colour-only line marks"
        );
    }

    #[test]
    fn count_labels() {
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
    fn dom_ids_are_built_from_keys() {
        assert_eq!(patch_dom_id("9-1"), "patch-9-1");
        assert_eq!(hero_dom_id("9-1", "d-va"), "patch-9-1-hero-d-va");
    }

    #[test]
    fn patch_notes_load_without_long_dashes() {
        let raw = r#"{"data": [{
            "version": "9.1",
            "date": "2026-08-11",
            "title": "Season 9 \u2014 Balance",
            "body": "Live August 11\u201331 \u2014 read on",
            "url": "https://example.com/patch",
            "hero_updates": [{
                "hero_id": "ana",
                "hero_name": "Ana",
                "dev_comment": "Too strong \u2014 toned down",
                "changes": [{
                    "ability": "Sleep \u2014 Dart",
                    "description": "Cooldown 12\u201314 seconds"
                }]
            }],
            "sections": [{
                "category": "General \u2014 Updates",
                "items": ["Mon\u2013Fri queue", "Fixed \u2014"]
            }]
        }]}"#;
        let list: ListResponse = serde_json::from_str(raw).expect("parse");
        let note = &list.data[0];
        assert_eq!(note.title.as_deref(), Some("Season 9, Balance"));
        assert_eq!(note.body.as_deref(), Some("Live August 11 to 31, read on"));
        let hero = &note.hero_updates[0];
        assert_eq!(hero.dev_comment.as_deref(), Some("Too strong, toned down"));
        assert_eq!(hero.changes[0].ability.as_deref(), Some("Sleep, Dart"));
        assert_eq!(hero.changes[0].description, "Cooldown 12 to 14 seconds");
        let section = &note.sections[0];
        assert_eq!(section.category, "General, Updates");
        assert_eq!(section.items, vec!["Mon to Fri queue", "Fixed"]);
        assert_eq!(note.version, "9.1");
        assert_eq!(note.date, "2026-08-11");
        assert_eq!(note.url, "https://example.com/patch");
    }

    #[test]
    fn patch_search_matches_the_cleaned_title() {
        let raw = r#"{"data": [{"version": "9.0", "title": "Season 9 \u2014 Balance"}]}"#;
        let list: ListResponse = serde_json::from_str(raw).expect("parse");
        assert!(patch_matches_search(&list.data[0], "season 9, balance"));
    }

    #[test]
    fn verdict_ink_clears_aa_on_its_fill() {
        // Badge ink is 11px / 600 text, so it needs 4.5:1. Accent or info ink
        // on a 12% wash of itself misses that in light, so every verdict sits
        // on an opaque soft token (or the plain surface) instead.
        for verdict in Verdict::ALL {
            let (ink, fill) = verdict.ink_and_fill();
            assert_ink_clears_aa(ink, fill, 4.5);
            let rule = PAGE_CSS
                .split(&format!(".pn-verdict.{} {{", verdict.tone_class()))
                .nth(1)
                .or_else(|| PAGE_CSS.split(".pn-verdict.is-other {").nth(1))
                .and_then(|rest| rest.split('}').next())
                .expect("verdict rule");
            assert!(
                has_decl(rule, &format!("color: var({ink})")),
                "{verdict:?}: {rule}"
            );
            assert!(
                has_decl(rule, &format!("background: var({fill})")),
                "{verdict:?}: {rule}"
            );
        }
        assert!(!PAGE_CSS.contains("color-mix"), "no washes on this page");
        // Section titles carry a start bar in the solid accent.
        assert!(has_decl(
            &css_rule(PAGE_CSS, "    .pn-section-title {\n        padding"),
            "border-inline-start: var(--space-1) solid var(--accent-solid)"
        ));
    }

    #[test]
    fn verdict_is_a_word_plus_a_shape() {
        assert_eq!(Verdict::from_change_type("buff"), Verdict::Buff);
        assert_eq!(Verdict::from_change_type("nerf"), Verdict::Nerf);
        assert_eq!(Verdict::from_change_type("adjustment"), Verdict::Adjust);
        assert_eq!(Verdict::from_change_type("bugfix"), Verdict::Fix);
        assert_eq!(Verdict::from_change_type(""), Verdict::Other);
        let labels: Vec<_> = Verdict::ALL.iter().map(|v| v.label()).collect();
        assert_eq!(labels, ["Buff", "Nerf", "Adjust", "Fix", "Other"]);

        let html = render(ready_app);
        assert!(
            html.contains(
                "<span class=\"pn-verdict is-buff\"><span class=\"pn-mark pn-mark-up\" aria-hidden=\"true\"></span>Buff</span>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<span class=\"pn-verdict is-fix\"><span class=\"pn-mark pn-mark-dot\" aria-hidden=\"true\"></span>Fix</span>"
            ),
            "{html}"
        );
        for shape in [
            ".pn-mark-up {\n        border-bottom: var(--space-2) solid currentColor;",
            ".pn-mark-down {\n        border-top: var(--space-2) solid currentColor;",
            ".pn-mark-diamond {\n        transform: rotate(45deg);",
            ".pn-mark-dot {\n        border-radius: var(--radius-pill);",
        ] {
            assert!(PAGE_CSS.contains(shape), "{shape}");
        }
    }

    #[test]
    fn page_has_one_h1_and_no_main() {
        for html in [
            render(ready_app),
            render(loading_app),
            render(failed_app),
            render(empty_app),
            render(no_match_app),
        ] {
            assert_eq!(html.matches("<h1").count(), 1, "{html}");
            assert!(html.contains("<h1>Patch notes</h1>"), "{html}");
            assert!(!html.contains("<main"), "{html}");
            assert!(
                html.contains("class=\"ui-shell ui-shell--narrow patch-page\""),
                "{html}"
            );
        }
    }

    #[test]
    fn lead_leads_with_the_count() {
        assert_eq!(
            page_lead(Some((2, 2, false))),
            format!("2 notes. {LEAD_REST}")
        );
        assert_eq!(
            page_lead(Some((1, 35, true))),
            format!("Showing 1 of 35. {LEAD_REST}")
        );
        assert_eq!(page_lead(None), LEAD_REST);
        let html = render(ready_app);
        assert!(
            html.contains("<p class=\"page-lead\">2 notes. Open a row"),
            "{html}"
        );
        // No count while there is nothing to count.
        for html in [render(loading_app), render(failed_app), render(empty_app)] {
            assert!(html.contains("<p class=\"page-lead\">Open a row"), "{html}");
        }
    }

    #[test]
    fn status_line_is_always_rendered_and_only_its_text_changes() {
        let line = |text: &str| {
            format!(
                "<p id=\"patch-notes-status\" class=\"visually-hidden\" role=\"status\">{text}</p>"
            )
        };
        for html in [render(ready_app), render(failed_app), render(empty_app)] {
            assert!(html.contains(&line("")), "{html}");
        }
        // Loading and a retry are said by this line, not by a second region.
        for html in [render(loading_app), render(retrying_app)] {
            assert!(html.contains(&line("Loading patch notes")), "{html}");
            assert_eq!(html.matches("role=\"status\"").count(), 1, "{html}");
        }
        assert!(
            render(search_hit_app).contains(&line("Showing 1 of 2.")),
            "search hits"
        );
        assert!(
            render(no_match_app).contains(&line(NO_MATCH_TEXT)),
            "no matches"
        );
        assert_eq!(status_text(PatchFetchState::Ready, false, 2, 2, false), "");
        assert_eq!(status_text(PatchFetchState::Ready, false, 0, 0, true), "");
        assert_eq!(
            status_text(PatchFetchState::Failed, true, 0, 0, false),
            "Loading patch notes"
        );
        assert_eq!(status_text(PatchFetchState::Failed, false, 0, 0, false), "");
    }

    #[test]
    fn retry_says_so_and_keeps_the_button_focusable() {
        let html = render(retrying_app);
        assert!(html.contains(">Retrying...</button>"), "{html}");
        assert!(html.contains("aria-disabled=\"true\""), "{html}");
        assert!(html.contains("aria-busy=\"true\""), "{html}");
        assert!(!html.contains(" disabled"), "{html}");
    }

    #[test]
    fn filter_chips_are_one_scrolling_group() {
        let html = render(ready_app);
        assert!(
            html.contains(
                "<div class=\"chip-row is-scroll\" role=\"group\" aria-label=\"Filter patch notes\">"
            ),
            "{html}"
        );
        // One chip per filter: the pressed one, then the rest.
        assert_eq!(html.matches("class=\"chip active\"").count(), 1, "{html}");
        assert_eq!(
            html.matches("class=\"chip\"").count(),
            FILTER_OPTIONS.len() - 1,
            "{html}"
        );
        assert_eq!(html.matches("aria-pressed=\"true\"").count(), 1, "{html}");
        assert_eq!(
            html.matches("aria-pressed=\"false\"").count(),
            FILTER_OPTIONS.len() - 1,
            "{html}"
        );
        assert!(
            html.contains(
                "<button type=\"button\" aria-pressed=\"true\" class=\"chip active\">All</button>"
            ),
            "{html}"
        );
        // "Competitive" shows "Comp" on phone; its name stays the full word.
        assert!(
            html.contains(
                "<button type=\"button\" aria-pressed=\"false\" aria-label=\"Competitive\" class=\"chip\"><span class=\"chip-full\">Competitive</span><span class=\"chip-short\">Comp</span></button>"
            ),
            "{html}"
        );
        assert_eq!(
            html.matches("aria-label=\"Competitive\"").count(),
            1,
            "{html}"
        );
        assert_eq!(
            &FILTER_OPTIONS[..6],
            ["All", "Latest", "Buffs", "Nerfs", "Fixes", "Adjusts"],
            "the frame's chips come first"
        );
        let latest = render(latest_app);
        assert_eq!(
            latest.matches("class=\"pn-row").count() - latest.matches("class=\"pn-row-").count(),
            1,
            "{latest}"
        );
        assert!(latest.contains("Showing 1 of 2."), "{latest}");
    }

    #[test]
    fn rows_expose_expanded_state_and_data_ids() {
        let html = render(ready_app);
        assert!(html.contains("<p class=\"pn-year\">2026</p>"), "{html}");
        assert!(html.contains("<p class=\"pn-year\">2025</p>"), "{html}");
        assert!(
            html.contains(
                "<div class=\"pn-row is-open\" id=\"patch-9-1\"><h2 class=\"pn-row-title\"><button class=\"pn-row-head\" type=\"button\" aria-expanded=\"true\" aria-controls=\"patch-9-1-body\">"
            ),
            "{html}"
        );
        assert!(
            html.contains("<div class=\"pn-row-body\" id=\"patch-9-1-body\">"),
            "{html}"
        );
        assert!(
            html.contains(
                "<div class=\"pn-row\" id=\"patch-8-0\"><h2 class=\"pn-row-title\"><button class=\"pn-row-head\" type=\"button\" aria-expanded=\"false\">"
            ),
            "{html}"
        );
        assert!(!html.contains("patch-8-0-body"), "{html}");
        // Ids come from the notes, never from their position in the list.
        assert!(!html.contains("id=\"patch-0"), "{html}");
        assert!(
            html.contains("<span class=\"pn-row-name\">9.1 \u{b7} Season 9 Hotfix</span> <span class=\"pn-row-meta\">Latest \u{b7} 2026-11-05 \u{b7} 2 heroes \u{b7} 3 changes</span>"),
            "{html}"
        );
        assert_eq!(html.matches(">Latest \u{b7}").count(), 1, "{html}");
        // Hero cards start closed, named by a heading, with their own state.
        assert!(
            html.contains(
                "<div class=\"pn-hero-card\" id=\"patch-9-1-hero-wrecking-ball\"><h3 class=\"pn-hero-title\"><button class=\"pn-hero-head\" type=\"button\" aria-expanded=\"false\">"
            ),
            "{html}"
        );
        assert!(
            html.contains("<h3 class=\"pn-section-title\">General</h3>"),
            "{html}"
        );
        // The hero group is named after its row, so two open rows differ.
        assert!(
            html.contains(
                "role=\"group\" aria-label=\"Hero changes in 9.1 \u{b7} Season 9 Hotfix\""
            ),
            "{html}"
        );
        // No control inside a link, no control inside a control.
        assert!(!html.contains("role=\"button\""), "{html}");
        assert!(html.contains(">Expand all</button>"), "{html}");
    }

    #[test]
    fn search_hits_and_no_matches_keep_the_toolbar() {
        let hit = render(search_hit_app);
        assert!(hit.contains("value=\"mercy\""), "{hit}");
        assert!(hit.contains("Season 8 Launch"), "{hit}");
        assert!(!hit.contains("Season 9 Hotfix"), "{hit}");
        assert!(
            !hit.contains("pn-expand"),
            "one row needs no Expand all: {hit}"
        );

        let none = render(no_match_app);
        assert!(none.contains("class=\"field-input pn-search\""), "{none}");
        assert!(none.contains("class=\"chip-row is-scroll\""), "{none}");
        // The shared empty card; the status line says it for assistive tech.
        assert!(
            none.contains(
                "<section class=\"empty-card\"><h2>No matching patch notes</h2><p>Try clearing search or switching the filter.</p></section>"
            ),
            "{none}"
        );
        assert!(!none.contains("pn-expand"), "{none}");
        assert!(!none.contains("class=\"pn-row"), "{none}");
    }

    #[test]
    fn loading_failed_and_empty_have_no_toolbar() {
        let loading = render(loading_app);
        assert!(loading.contains(">Loading patch notes</p>"), "{loading}");
        assert!(
            loading.contains("<div class=\"ui-skel-list\" aria-hidden=\"true\">"),
            "{loading}"
        );
        let failed = render(failed_app);
        assert!(
            failed.contains("load patch notes. Check your connection"),
            "{failed}"
        );
        assert!(failed.contains("role=\"alert\""), "{failed}");
        assert!(failed.contains("fetch-error__retry"), "{failed}");
        let empty = render(empty_app);
        assert!(
            empty.contains("<section class=\"empty-card\"><h2>No patch notes yet</h2>"),
            "{empty}"
        );
        for html in [loading, failed, empty] {
            assert!(!html.contains("pn-search"), "{html}");
            assert!(!html.contains("chip-row"), "{html}");
        }
    }

    #[test]
    fn css_sizes_come_from_tokens_and_targets_are_44() {
        assert_sizes_from_tokens(PAGE_CSS);
        for (rule, decl) in [
            (".pn-search {", "height: var(--hit-target-min)"),
            (
                ".pn-hero-head {\n        box-sizing",
                "min-height: var(--hit-target-min)",
            ),
            (".pn-external-link {", "min-height: var(--hit-target-min)"),
        ] {
            assert!(has_decl(&css_rule(PAGE_CSS, rule), decl), "{rule}");
        }
        // Expand all, Retry, the search look and the jump chips are the
        // shared 44px controls; the page keeps no copy of them.
        for gone in [
            "fetch-error__retry",
            "min-height: var(--button-height)",
            ".pn-sr",
            ".pn-empty",
        ] {
            assert!(!PAGE_CSS.contains(gone), "{gone} is shared now");
        }
        let html = render(ready_app);
        assert!(
            html.contains("<button class=\"btn is-ghost pn-expand\" type=\"button\""),
            "{html}"
        );
        // Row headers and hero summaries share the 44px rule.
        assert!(PAGE_CSS.contains("    .pn-row-head,\n    .pn-hero-head {\n        box-sizing"));
        // Density tokens, so data-density applies.
        assert!(has_decl(
            &css_rule(PAGE_CSS, ".pn-row-head {\n        display"),
            "padding: var(--space-card-pad)"
        ));
        assert!(has_decl(
            &css_rule(PAGE_CSS, ".pn-list {"),
            "gap: var(--space-section)"
        ));
        assert!(has_decl(
            &css_rule(PAGE_CSS, ".pn-year-block {"),
            "gap: var(--space-stack)"
        ));
        // The toolbar is opaque where it sticks.
        assert!(has_decl(
            &css_rule(PAGE_CSS, ".pn-controls {"),
            "background: var(--bg)"
        ));
    }

    /// Whatever takes focus or is scrolled to under the sticky controls lands
    /// below them: row headers, hero headers, jump chips, the external link
    /// and the hero card itself.
    #[test]
    fn focus_targets_clear_the_sticky_controls() {
        let offset = css_rule(PAGE_CSS, ".patch-page {");
        assert!(
            offset.contains("--pn-sticky-offset: calc(")
                && offset.contains("var(--nav-height-used) + var(--hit-target-min) * 2"),
            "{offset}"
        );
        for marker in [
            ".pn-row-head,\n    .pn-hero-head {\n        box-sizing",
            ".pn-toc-chip {",
            ".pn-hero-card {",
            ".pn-external-link {",
        ] {
            assert!(
                has_decl(
                    &css_rule(PAGE_CSS, marker),
                    "scroll-margin-top: var(--pn-sticky-offset)"
                ),
                "{marker}"
            );
        }
    }

    #[test]
    fn external_link_says_where_it_goes_and_that_it_opens_a_tab() {
        let html = render(two_open_app);
        assert!(
            html.contains(
                "aria-label=\"View official patch notes for 9.1 \u{b7} Season 9 Hotfix (opens in a new tab)\">View official patch notes <span aria-hidden=\"true\">\u{2192}</span></a>"
            ),
            "{html}"
        );
        assert!(html.contains("target=\"_blank\""), "{html}");
        assert!(html.contains("rel=\"noopener noreferrer\""), "{html}");
    }
}
