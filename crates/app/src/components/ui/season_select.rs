//! Season picker shared by My Stats, member stats, and leaderboards.
//!
//! The control offers "All time", an explicit "Current season", and each
//! season from `GET /api/public/seasons`. The select is omitted while that
//! list is loading or empty, and a retry is shown when the request fails.
//! A visible label still renders in those states, without `for`, because
//! there is no select to point at.
//!
//! Nothing saved means all time. The "Current season" option stores
//! [`CURRENT_SEASON`] so a later rollover follows. Every season row is pinned
//! by id, including the season that is current now. The sentinel is kept when
//! no season is current, and that option says none is running. A saved id the
//! list cannot resolve becomes all time and is removed. An id saved by the
//! previous picker is migrated once.

use dioxus::prelude::*;
use scuffed_types::Season;

use crate::components::ui::label::{field_aria_label, nonempty};
use crate::components::ui::{BtnSize, BtnVariant, Button, Label};
#[cfg(not(test))]
use crate::hooks::use_api;
use crate::util::season_url;

pub const ALL_TIME_LABEL: &str = "All time";

/// Live `stats-season-v2` value meaning "whichever season is current right now".
const CURRENT_SEASON: &str = "current";

const CURRENT_SEASON_LABEL: &str = "Current season";
const CURRENT_SEASON_GAP_LABEL: &str = "Current season (none running, showing all time)";

/// Previous picker key (`stats-season`). The memo and the effect read it on
/// every run until a loaded list migrates it and the key is removed.
const LEGACY_SEASON_KEY: &str = "stats-season";

/// Live key. `current` follows the season flagged current; any other value is
/// a pinned id.
const STATS_SEASON_KEY: &str = "stats-season-v2";

pub const SEASON_SELECT_CSS: &str = r#"
.season-select { display: flex; flex-direction: column; gap: var(--space-1); }
.season-select-status { margin: 0; color: var(--text-3); font-size: var(--text-xs); }
"#;

/// One season the picker can resolve a saved value against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SeasonChoice<'a> {
    id: &'a str,
    is_current: bool,
}

/// Trimmed, non-empty storage or picker token.
fn token(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

/// Resolve a stored value against the loaded season list.
///
/// - missing or blank storage → `None` (all time)
/// - [`CURRENT_SEASON`] → the first id with `is_current`, or `None` when none is current
/// - any other string → that id when it is in `seasons`, otherwise `None`
fn resolve_stored_season<'a>(
    stored: Option<&str>,
    seasons: &[SeasonChoice<'a>],
) -> Option<&'a str> {
    let stored = token(stored)?;
    if stored == CURRENT_SEASON {
        return seasons.iter().find(|s| s.is_current).map(|s| s.id);
    }
    seasons.iter().find(|s| s.id == stored).map(|s| s.id)
}

/// What to write for a picker choice. `None` clears storage (all time).
///
/// Only the explicit "Current season" option stores the sentinel. Every
/// season row is pinned to its own id, including the row that is current
/// now, so that click does not jump the control onto "Current season".
fn season_to_store(selected: Option<&str>) -> Option<&str> {
    token(selected)
}

/// One-shot rewrite of an id saved by the previous picker.
///
/// A legacy `current` stays `current`. The id of the season marked current
/// becomes `current`. Any other listed id is kept. Anything else is dropped.
///
/// An id migrated while no season is current stays pinned. The next season
/// starting does not turn that pin into follow. Nothing later says the old
/// picker user meant to follow rather than stay on that season.
fn migrate_legacy(legacy: &str, seasons: &[SeasonChoice<'_>]) -> Option<String> {
    let legacy = token(Some(legacy))?;
    if legacy == CURRENT_SEASON
        || resolve_stored_season(Some(CURRENT_SEASON), seasons) == Some(legacy)
    {
        Some(CURRENT_SEASON.to_string())
    } else if seasons.iter().any(|s| s.id == legacy) {
        Some(legacy.to_string())
    } else {
        None
    }
}

/// Saved value that the loaded list cannot turn into a season id.
/// The sentinel is never stale: a gap with no current season still follows.
fn stored_season_is_stale(stored: Option<&str>, seasons: &[SeasonChoice<'_>]) -> bool {
    match token(stored) {
        None | Some(CURRENT_SEASON) => false,
        Some(value) => resolve_stored_season(Some(value), seasons).is_none(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedSeason {
    /// A saved value is present and the season list has not arrived.
    Pending,
    All,
    Id(String),
}

#[derive(Clone, Copy)]
enum SeasonList<'a> {
    Loading,
    /// The seasons request finished without a list.
    Unavailable,
    Ready(&'a [SeasonChoice<'a>]),
}

/// Map storage plus the list's load state onto the filter stats requests use.
fn resolve_season_filter(stored: Option<&str>, list: SeasonList<'_>) -> ResolvedSeason {
    match list {
        SeasonList::Loading => {
            if token(stored).is_some() {
                ResolvedSeason::Pending
            } else {
                ResolvedSeason::All
            }
        }
        SeasonList::Unavailable => ResolvedSeason::All,
        SeasonList::Ready(seasons) => match resolve_stored_season(stored, seasons) {
            Some(id) => ResolvedSeason::Id(id.to_string()),
            None => ResolvedSeason::All,
        },
    }
}

/// Live key wins. A legacy id is held until the list can migrate it.
fn resolve_saved(v2: Option<&str>, legacy: Option<&str>, list: SeasonList<'_>) -> ResolvedSeason {
    if token(v2).is_some() {
        return resolve_season_filter(v2, list);
    }
    match list {
        SeasonList::Loading => {
            if token(legacy).is_some() {
                ResolvedSeason::Pending
            } else {
                ResolvedSeason::All
            }
        }
        SeasonList::Unavailable => ResolvedSeason::All,
        SeasonList::Ready(seasons) => {
            let migrated = legacy.and_then(|value| migrate_legacy(value, seasons));
            resolve_season_filter(migrated.as_deref(), SeasonList::Ready(seasons))
        }
    }
}

/// In-session `Id(current)` follows the list. A pinned id is sent as itself.
/// `Some(Pending)` is representable, but [`StatsSeason::choose`] never writes it.
fn apply_choice(
    choice: Option<&ResolvedSeason>,
    v2: Option<&str>,
    legacy: Option<&str>,
    list: SeasonList<'_>,
) -> ResolvedSeason {
    match choice {
        Some(ResolvedSeason::All) => ResolvedSeason::All,
        Some(ResolvedSeason::Id(id)) if token(Some(id)) == Some(CURRENT_SEASON) => {
            resolve_season_filter(Some(CURRENT_SEASON), list)
        }
        Some(ResolvedSeason::Id(id)) => ResolvedSeason::Id(id.clone()),
        Some(ResolvedSeason::Pending) | None => resolve_saved(v2, legacy, list),
    }
}

/// Select token computed with the request filter, so a render does not read
/// storage on its own. The sentinel stays `current` instead of the live id.
fn select_for(
    choice: Option<&ResolvedSeason>,
    v2: Option<&str>,
    legacy: Option<&str>,
    list: SeasonList<'_>,
) -> Option<String> {
    match choice {
        Some(ResolvedSeason::All) | Some(ResolvedSeason::Pending) => None,
        Some(ResolvedSeason::Id(id)) => Some(id.clone()),
        None => stored_select(v2, legacy, list),
    }
}

fn stored_select(v2: Option<&str>, legacy: Option<&str>, list: SeasonList<'_>) -> Option<String> {
    let SeasonList::Ready(seasons) = list else {
        return None;
    };
    if seasons.is_empty() {
        return None;
    }
    if let Some(v2) = token(v2) {
        return select_token(v2, seasons);
    }
    legacy.and_then(|value| migrate_legacy(value, seasons))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SeasonView {
    request: ResolvedSeason,
    select: Option<String>,
}

fn season_view(
    choice: Option<&ResolvedSeason>,
    v2: Option<&str>,
    legacy: Option<&str>,
    list: SeasonList<'_>,
) -> SeasonView {
    SeasonView {
        request: apply_choice(choice, v2, legacy, list),
        select: select_for(choice, v2, legacy, list),
    }
}

/// Request path for a resolved filter. Pending yields an empty path so callers
/// skip the fetch instead of sending `season=current` or an unchecked id.
fn season_filter_path(resolved: &ResolvedSeason, path: &str) -> String {
    match resolved {
        ResolvedSeason::Pending => String::new(),
        ResolvedSeason::All => path.to_string(),
        ResolvedSeason::Id(id) => season_url(path, Some(id.as_str())),
    }
}

/// `<select>` value for a stored token once the list is on screen.
/// The sentinel stays `current` so it does not highlight a pinned row.
fn select_token(stored: &str, seasons: &[SeasonChoice<'_>]) -> Option<String> {
    let stored = token(Some(stored))?;
    if stored == CURRENT_SEASON {
        return Some(CURRENT_SEASON.to_string());
    }
    resolve_stored_season(Some(stored), seasons).map(str::to_string)
}

/// Shared stats-season filter. Copy it into each fetch closure.
///
/// [`Self::fetch_path`] reads only the resolved memo. Reading
/// [`Self::season_list`] inside a fetch closure subscribes that fetch to the
/// season list and refetches whenever the list updates.
#[derive(Clone, Copy)]
pub struct StatsSeason {
    view: Memo<SeasonView>,
    /// `Resource` is `Copy` without requiring `Season: Copy`. `ApiResource` is
    /// not, because its derive adds that bound.
    rows: Resource<Option<Vec<Season>>>,
    refresh: Signal<u64>,
    error: Signal<Option<String>>,
    /// `None` means "use storage". `Some` is a choice made in this session.
    /// `Some(Pending)` is representable, but [`Self::choose`] never writes it.
    /// `Id(current)` follows; any other id is pinned.
    pick: Signal<Option<ResolvedSeason>>,
    /// Bumps when another document writes `stats-season-v2` or clears storage.
    /// localStorage itself is not a signal.
    storage_rev: Signal<u64>,
}

impl StatsSeason {
    fn resolved(self) -> ResolvedSeason {
        self.view.read().request.clone()
    }

    /// Value for the `<select>`.
    ///
    /// Comes from the same memo as the request filter, so a render does not
    /// read storage itself. `None` is all time, and also while a saved value
    /// is still unresolved (the picker stays hidden until the list loads).
    /// `Some("current")` follows the current season. `Some(id)` is pinned.
    pub fn selected_id(self) -> Option<String> {
        self.view.read().select.clone()
    }

    /// Handle for the shared `/api/public/seasons` fetch. The picker reads it;
    /// stats requests must use [`Self::fetch_path`] so they do not subscribe
    /// to the list.
    pub fn season_list(self) -> Resource<Option<Vec<Season>>> {
        self.rows
    }

    /// Last seasons-fetch error. `None` while loading or after a success.
    pub fn seasons_error(self) -> Signal<Option<String>> {
        self.error
    }

    /// Ask for the season list again after a failed fetch.
    pub fn retry(mut self) {
        self.refresh += 1;
    }

    /// Path to fetch, or `""` while a saved season is still unresolved.
    /// `use_api_with` treats an empty path as "do not send".
    pub fn fetch_path(self, path: &str) -> String {
        season_filter_path(&self.resolved(), path)
    }

    /// Record a picker change. `None` is all time. The stored token and the
    /// in-session pick are the same trimmed value. Only the explicit
    /// "Current season" option stores the sentinel; a season row stores its id.
    pub fn choose(mut self, picked: Option<String>) {
        let stored = season_to_store(picked.as_deref()).map(str::to_string);
        write_v2(stored.as_deref());
        write_legacy(None);
        let next = match stored.as_deref() {
            None => Some(ResolvedSeason::All),
            Some(id) => Some(ResolvedSeason::Id(id.to_string())),
        };
        // Always write. An equal value still notifies, so a click the browser
        // just highlighted is painted back. The memo's PartialEq keeps an
        // unchanged filter from refetching.
        self.pick.set(next);
    }

    #[cfg(test)]
    fn apply_storage_event(self, key: Option<&str>) {
        note_season_storage(key, self.pick, self.storage_rev);
    }
}

fn choices_from(list: &[Season]) -> Vec<SeasonChoice<'_>> {
    list.iter()
        .map(|s| SeasonChoice {
            id: s.id.as_str(),
            is_current: s.is_current,
        })
        .collect()
}

fn read_v2() -> Option<String> {
    storage_get(STATS_SEASON_KEY)
}

fn read_legacy() -> Option<String> {
    storage_get(LEGACY_SEASON_KEY)
}

fn write_v2(value: Option<&str>) {
    storage_set(STATS_SEASON_KEY, value);
}

fn write_legacy(value: Option<&str>) {
    storage_set(LEGACY_SEASON_KEY, value);
}

#[cfg(test)]
fn storage_get(key: &str) -> Option<String> {
    let raw = TEST_STORAGE.with(|slot| slot.borrow().get(key).cloned());
    token(raw.as_deref()).map(str::to_string)
}

#[cfg(not(test))]
fn storage_get(key: &str) -> Option<String> {
    #[cfg(feature = "web")]
    {
        if let Some(win) = web_sys::window()
            && let Ok(Some(storage)) = win.local_storage()
            && let Ok(Some(v)) = storage.get_item(key)
        {
            return token(Some(v.as_str())).map(str::to_string);
        }
    }
    #[cfg(not(feature = "web"))]
    let _ = key;
    None
}

#[cfg(test)]
fn storage_set(key: &str, value: Option<&str>) {
    TEST_STORAGE.with(|slot| {
        let mut map = slot.borrow_mut();
        match token(value) {
            Some(v) => {
                map.insert(key.to_string(), v.to_string());
            }
            None => {
                map.remove(key);
            }
        }
    });
}

#[cfg(not(test))]
fn storage_set(key: &str, value: Option<&str>) {
    #[cfg(feature = "web")]
    {
        if let Some(win) = web_sys::window()
            && let Ok(Some(storage)) = win.local_storage()
        {
            let _ = match token(value) {
                Some(v) => storage.set_item(key, v),
                None => storage.remove_item(key),
            };
        }
    }
    #[cfg(not(feature = "web"))]
    let _ = (key, value);
}

#[cfg(test)]
#[derive(Clone)]
enum SeasonFetch {
    Hold,
    Ready(Vec<Season>),
    Failed,
}

#[cfg(test)]
thread_local! {
    static TEST_STORAGE: std::cell::RefCell<std::collections::HashMap<String, String>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    static TEST_SEASONS: std::cell::RefCell<SeasonFetch> =
        const { std::cell::RefCell::new(SeasonFetch::Hold) };
    static TEST_PROBE: std::cell::Cell<Option<StatsSeason>> = const { std::cell::Cell::new(None) };
}

/// `storage` handler. A matching key drops the in-session pick and bumps
/// `rev` so the memo re-reads localStorage. Other keys are ignored.
/// A null key is `localStorage.clear()`.
fn note_season_storage(
    key: Option<&str>,
    mut pick: Signal<Option<ResolvedSeason>>,
    mut rev: Signal<u64>,
) {
    if storage_event_targets_season(key) {
        pick.set(None);
        rev += 1;
    }
}

fn storage_event_targets_season(key: Option<&str>) -> bool {
    match key {
        None => true,
        Some(key) => key == STATS_SEASON_KEY,
    }
}

/// Listen for `stats-season-v2` writes from another tab.
///
/// The `storage` event does not fire in the document that wrote the key.
/// Installed once and removed on unmount (`use_drop`). Do not `Closure::forget`.
/// `Closure::wrap` aborts off wasm32. Native tests call [`note_season_storage`]
/// and skip this listener.
#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn use_season_storage_sync(mut on_key: impl FnMut(Option<&str>) + 'static) {
    use std::rc::Rc;

    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    type StorageHandler = Closure<dyn FnMut(web_sys::StorageEvent)>;

    fn listener_fn(closure: &StorageHandler) -> &js_sys::Function {
        closure.as_ref().unchecked_ref()
    }

    let listener = use_hook(move || {
        let closure: Rc<StorageHandler> = Rc::new(Closure::wrap(Box::new(
            move |event: web_sys::StorageEvent| {
                let key = event.key();
                on_key(key.as_deref());
            },
        )
            as Box<dyn FnMut(web_sys::StorageEvent)>));
        if let Some(window) = web_sys::window() {
            let _ = window.add_event_listener_with_callback("storage", listener_fn(&closure));
        }
        closure
    });
    let listener = listener.clone();
    use_drop(move || {
        if let Some(window) = web_sys::window() {
            let _ = window.remove_event_listener_with_callback("storage", listener_fn(&listener));
        }
    });
}

/// `Closure::wrap` aborts off wasm32, so the listener is not installed.
#[cfg(not(all(feature = "web", target_arch = "wasm32")))]
fn use_season_storage_sync(_on_key: impl FnMut(Option<&str>) + 'static) {}

/// Shared season filter for My Stats, member stats, and leaderboards.
///
/// Call it once, unconditionally, at the top of the page. The three pages
/// share `stats-season-v2`, so a pick on one page is the pick on the others.
pub fn use_stats_season() -> StatsSeason {
    let pick = use_signal(|| None::<ResolvedSeason>);
    let storage_rev = use_signal(|| 0u64);
    let (rows, refresh, error) = use_season_rows();
    // The memo is the only signal stats resources should read. It notifies
    // only when PartialEq says the view changed, so nothing saved stays
    // one all-time fetch after the season list arrives. The select token
    // rides along so a render does not read storage on its own.
    // `storage_rev` is how another tab's `storage` event gets into this memo.
    let view = use_memo(move || {
        let _storage_rev = storage_rev();
        let choice = pick();
        let v2 = read_v2();
        let legacy = read_legacy();
        let data = rows.read();
        match data.as_ref() {
            Some(Some(list)) => {
                let choices = choices_from(list);
                season_view(
                    choice.as_ref(),
                    v2.as_deref(),
                    legacy.as_deref(),
                    SeasonList::Ready(&choices),
                )
            }
            Some(None) => season_view(
                choice.as_ref(),
                v2.as_deref(),
                legacy.as_deref(),
                SeasonList::Unavailable,
            ),
            None => season_view(
                choice.as_ref(),
                v2.as_deref(),
                legacy.as_deref(),
                SeasonList::Loading,
            ),
        }
    });

    // `pick` means the user chose in this session, so this effect must not
    // rewrite storage out from under that choice. localStorage is not a
    // signal; the memo already resolved this pass.
    use_effect(move || {
        if pick().is_some() {
            return;
        }
        let data = rows.read();
        let Some(list) = data.as_ref().and_then(|inner| inner.as_ref()) else {
            return;
        };
        let choices = choices_from(list);
        if read_v2().is_none() {
            if let Some(legacy) = read_legacy() {
                write_v2(migrate_legacy(&legacy, &choices).as_deref());
                write_legacy(None);
                return;
            }
        } else if read_legacy().is_some() {
            write_legacy(None);
        }
        if stored_season_is_stale(read_v2().as_deref(), &choices) {
            write_v2(None);
        }
    });

    use_season_storage_sync(move |key| {
        note_season_storage(key, pick, storage_rev);
    });

    StatsSeason {
        view,
        rows,
        refresh,
        error,
        pick,
        storage_rev,
    }
}

type SeasonRows = (
    Resource<Option<Vec<Season>>>,
    Signal<u64>,
    Signal<Option<String>>,
);

#[cfg(not(test))]
fn use_season_rows() -> SeasonRows {
    let api = use_api::<Vec<Season>>("/api/public/seasons");
    (api.data, api.refresh, api.error)
}

#[cfg(test)]
fn use_season_rows() -> SeasonRows {
    let refresh = use_signal(|| 0u64);
    let mut error = use_signal(|| Option::<String>::None);
    let data = use_resource(move || {
        let _generation = refresh();
        async move {
            // Same moment as `use_api`: clear the error when the fetch starts,
            // while the previous `Some(None)` stays. The retry control stays
            // mounted for that gap.
            error.set(None);
            match TEST_SEASONS.with(|slot| slot.borrow().clone()) {
                SeasonFetch::Hold => {
                    std::future::pending::<()>().await;
                    None
                }
                SeasonFetch::Ready(list) => Some(list),
                SeasonFetch::Failed => {
                    error.set(Some("offline".into()));
                    None
                }
            }
        }
    });
    (data, refresh, error)
}

#[component]
pub fn SeasonSelect(
    /// Selected token. `None` or empty is all time, `"current"` follows, and
    /// any other string is a pinned season id.
    value: Option<String>,
    /// Fired on change. `None` = all time; `Some(id)` = that option's value.
    onchange: EventHandler<Option<String>>,
    /// Shared seasons fetch. The control subscribes by reading it.
    seasons: Resource<Option<Vec<Season>>>,
    /// Seasons-fetch error. Drives the retry affordance.
    seasons_error: Signal<Option<String>>,
    /// Bumps the seasons fetch after a failure.
    on_retry: EventHandler<()>,
    /// Optional `id` on the underlying `<select>`.
    #[props(default)]
    id: Option<String>,
    /// Optional `name` on the underlying `<select>`.
    #[props(default)]
    name: Option<String>,
    /// Optional visible label rendered above the control.
    #[props(default)]
    label: Option<String>,
) -> Element {
    let current = value.unwrap_or_default();
    let field_id = nonempty(id);
    let field_name = nonempty(name);
    let label_text = nonempty(label);
    let aria_label = field_aria_label(label_text.as_deref(), field_id.as_deref(), "Season");
    let data = seasons.read();
    let failed = seasons_error.read().is_some();
    match data.as_ref() {
        Some(Some(list)) if !list.is_empty() => {
            let follow_label = if list.iter().any(|s| s.is_current) {
                CURRENT_SEASON_LABEL
            } else {
                CURRENT_SEASON_GAP_LABEL
            };
            let label_for = field_id.clone();
            let shown_label = label_text.clone();
            rsx! {
                div { class: "season-select",
                    if let Some(text) = shown_label {
                        Label { for_id: label_for, {text} }
                    }
                    select {
                        class: "ui-field",
                        id: field_id,
                        name: field_name,
                        aria_label,
                        value: "{current}",
                        onchange: move |e| {
                            let v = e.value();
                            onchange.call(if v.is_empty() { None } else { Some(v) });
                        },
                        option {
                            key: "all",
                            value: "",
                            selected: current.is_empty(),
                            "{ALL_TIME_LABEL}"
                        }
                        option {
                            key: "follow-current",
                            value: "current",
                            selected: current == CURRENT_SEASON,
                            "{follow_label}"
                        }
                        for s in list.iter() {
                            option {
                                key: "{s.id}",
                                value: "{s.id}",
                                selected: current == s.id,
                                if s.is_current { "{s.name} (current)" } else { "{s.name}" }
                            }
                        }
                    }
                }
            }
        }
        // `Some(None)` is a finished failure, or a retry whose error was
        // cleared while the previous empty result is still held. Keep one
        // button mounted so keyboard focus is not dropped between them.
        Some(None) => {
            let retrying = !failed;
            let shown_label = label_text;
            rsx! {
                div { class: "season-select",
                    if let Some(text) = shown_label {
                        Label { {text} }
                    }
                    p { class: "season-select-status", "Couldn't load seasons." }
                    Button {
                        variant: BtnVariant::Ghost,
                        size: BtnSize::Sm,
                        disabled: retrying,
                        onclick: move |_| on_retry.call(()),
                        if retrying { "Retrying..." } else { "Retry" }
                    }
                }
            }
        }
        _ => rsx! {
            if let Some(text) = label_text {
                div { class: "season-select",
                    Label { {text} }
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::dioxus_core::{AttributeValue, Mutation, Mutations};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    const ROLE_PATH: &str = "/api/stats/me/roles";

    thread_local! {
        static RENDER_PATHS: std::cell::RefCell<Vec<String>> =
            const { std::cell::RefCell::new(Vec::new()) };
        static EFFECT_PATHS: std::cell::RefCell<Vec<String>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    fn during_season_four() -> Vec<SeasonChoice<'static>> {
        vec![
            SeasonChoice {
                id: "season-4",
                is_current: true,
            },
            SeasonChoice {
                id: "season-3",
                is_current: false,
            },
        ]
    }

    fn after_rollover() -> Vec<SeasonChoice<'static>> {
        vec![
            SeasonChoice {
                id: "season-5",
                is_current: true,
            },
            SeasonChoice {
                id: "season-4",
                is_current: false,
            },
        ]
    }

    /// API order is `starts_at DESC`, so an upcoming season can lead the list.
    fn upcoming_then_current() -> Vec<SeasonChoice<'static>> {
        vec![
            SeasonChoice {
                id: "season-6",
                is_current: false,
            },
            SeasonChoice {
                id: "season-5",
                is_current: true,
            },
            SeasonChoice {
                id: "season-4",
                is_current: false,
            },
        ]
    }

    #[test]
    fn sentinel_resolves_to_the_current_season() {
        assert_eq!(
            resolve_stored_season(Some(CURRENT_SEASON), &during_season_four()),
            Some("season-4")
        );
    }

    #[test]
    fn sentinel_after_rollover_resolves_to_the_new_current_season() {
        assert_eq!(
            resolve_stored_season(Some(CURRENT_SEASON), &after_rollover()),
            Some("season-5")
        );
    }

    #[test]
    fn sentinel_resolves_to_current_not_the_first_row() {
        assert_eq!(
            resolve_stored_season(Some(CURRENT_SEASON), &upcoming_then_current()),
            Some("season-5")
        );
    }

    #[test]
    fn pinned_past_id_stays_pinned() {
        assert_eq!(
            resolve_stored_season(Some("season-4"), &after_rollover()),
            Some("season-4")
        );
    }

    #[test]
    fn unknown_id_falls_back_to_none() {
        assert_eq!(
            resolve_stored_season(Some("missing"), &after_rollover()),
            None
        );
    }

    #[test]
    fn sentinel_with_no_current_season_falls_back_to_none() {
        let seasons = [SeasonChoice {
            id: "season-4",
            is_current: false,
        }];
        assert_eq!(resolve_stored_season(Some(CURRENT_SEASON), &seasons), None);
    }

    #[test]
    fn empty_storage_gives_none() {
        let seasons = after_rollover();
        assert_eq!(resolve_stored_season(None, &seasons), None);
        assert_eq!(resolve_stored_season(Some(""), &seasons), None);
        assert_eq!(resolve_stored_season(Some("   "), &seasons), None);
    }

    #[test]
    fn current_season_constant_is_the_select_value() {
        assert_eq!(CURRENT_SEASON, "current");
    }

    #[test]
    fn picking_a_season_row_stores_that_id() {
        assert_eq!(season_to_store(Some("season-4")), Some("season-4"));
        assert_eq!(season_to_store(Some("season-6")), Some("season-6"));
        assert_eq!(season_to_store(Some(CURRENT_SEASON)), Some(CURRENT_SEASON));
        let after = after_rollover();
        assert_eq!(
            resolve_stored_season(Some(CURRENT_SEASON), &after),
            Some("season-5")
        );
    }

    #[test]
    fn picking_a_past_season_stores_that_id() {
        assert_eq!(season_to_store(Some("season-4")), Some("season-4"));
    }

    #[test]
    fn second_current_season_is_pinned_not_the_sentinel() {
        let seasons = [
            SeasonChoice {
                id: "season-6",
                is_current: true,
            },
            SeasonChoice {
                id: "season-5",
                is_current: true,
            },
        ];
        assert_eq!(
            resolve_stored_season(Some(CURRENT_SEASON), &seasons),
            Some("season-6")
        );
        assert_eq!(season_to_store(Some("season-5")), Some("season-5"));
        assert_eq!(season_to_store(Some("season-6")), Some("season-6"));
        assert_eq!(season_to_store(Some(CURRENT_SEASON)), Some(CURRENT_SEASON));
    }

    #[test]
    fn picking_all_time_clears_storage() {
        assert_eq!(season_to_store(None), None);
        assert_eq!(season_to_store(Some("  ")), None);
    }

    #[test]
    fn stored_tokens_are_trimmed_once() {
        assert_eq!(season_to_store(Some("  season-4  ")), Some("season-4"));
        assert_eq!(
            resolve_stored_season(Some("  current  "), &during_season_four()),
            Some("season-4")
        );
    }

    #[test]
    fn legacy_current_id_becomes_the_sentinel_and_other_ids_stay() {
        let during = during_season_four();
        assert_eq!(
            migrate_legacy("season-4", &during),
            Some(CURRENT_SEASON.to_string())
        );
        assert_eq!(
            migrate_legacy("season-3", &during),
            Some("season-3".to_string())
        );
        assert_eq!(migrate_legacy("missing", &during), None);
        assert_eq!(
            migrate_legacy(CURRENT_SEASON, &during),
            Some(CURRENT_SEASON.to_string())
        );
        let gap = [SeasonChoice {
            id: "season-4",
            is_current: false,
        }];
        assert_eq!(
            migrate_legacy(CURRENT_SEASON, &gap),
            Some(CURRENT_SEASON.to_string())
        );
        // A listed id migrated while nothing is current stays that id.
        assert_eq!(
            migrate_legacy("season-4", &gap),
            Some("season-4".to_string())
        );
        let after = after_rollover();
        assert_eq!(
            resolve_stored_season(migrate_legacy("season-4", &during).as_deref(), &after),
            Some("season-5")
        );
    }

    #[test]
    fn empty_storage_is_all_time_before_the_list_loads() {
        let resolved = resolve_season_filter(None, SeasonList::Loading);
        assert_eq!(resolved, ResolvedSeason::All);
        assert_eq!(
            season_filter_path(&resolved, "/api/stats/me"),
            "/api/stats/me"
        );
    }

    #[test]
    fn saved_value_is_held_until_the_list_loads() {
        for stored in [CURRENT_SEASON, "season-4"] {
            let resolved = resolve_season_filter(Some(stored), SeasonList::Loading);
            assert_eq!(resolved, ResolvedSeason::Pending);
            assert_eq!(season_filter_path(&resolved, "/api/stats/me"), "");
        }
    }

    #[test]
    fn resolved_sentinel_path_uses_the_live_id() {
        let seasons = [SeasonChoice {
            id: "season-5",
            is_current: true,
        }];
        let resolved = resolve_season_filter(Some(CURRENT_SEASON), SeasonList::Ready(&seasons));
        assert_eq!(
            season_filter_path(&resolved, "/api/stats/me"),
            "/api/stats/me?season=season-5"
        );
    }

    #[test]
    fn unavailable_list_does_not_send_the_saved_value() {
        let resolved = resolve_season_filter(Some(CURRENT_SEASON), SeasonList::Unavailable);
        assert_eq!(resolved, ResolvedSeason::All);
        assert_eq!(
            season_filter_path(&resolved, "/api/stats/me"),
            "/api/stats/me"
        );
        let resolved = resolve_season_filter(Some("season-4"), SeasonList::Unavailable);
        assert_eq!(
            season_filter_path(&resolved, "/api/stats/me/heroes"),
            "/api/stats/me/heroes"
        );
    }

    #[test]
    fn stale_id_is_all_time_once_the_list_is_ready() {
        let seasons = after_rollover();
        let resolved = resolve_season_filter(Some("season-1"), SeasonList::Ready(&seasons));
        assert_eq!(resolved, ResolvedSeason::All);
        let path = season_filter_path(&resolved, ROLE_PATH);
        assert_eq!(path, ROLE_PATH);
        assert!(!path.contains("season="));
    }

    #[test]
    fn stale_storage_is_only_an_unresolved_value() {
        let seasons = after_rollover();
        let no_current = [SeasonChoice {
            id: "season-4",
            is_current: false,
        }];
        assert!(stored_season_is_stale(Some("missing"), &seasons));
        assert!(stored_season_is_stale(Some("missing"), &[]));
        assert!(!stored_season_is_stale(Some(CURRENT_SEASON), &[]));
        assert!(!stored_season_is_stale(Some(CURRENT_SEASON), &no_current));
        assert!(!stored_season_is_stale(None, &seasons));
        assert!(!stored_season_is_stale(Some(""), &seasons));
        assert!(!stored_season_is_stale(Some(CURRENT_SEASON), &seasons));
        assert!(!stored_season_is_stale(Some("season-4"), &seasons));
    }

    fn blank_hooks() {
        TEST_STORAGE.with(|slot| slot.borrow_mut().clear());
        TEST_SEASONS.with(|slot| *slot.borrow_mut() = SeasonFetch::Hold);
        RENDER_PATHS.with(|slot| slot.borrow_mut().clear());
        EFFECT_PATHS.with(|slot| slot.borrow_mut().clear());
        TEST_PROBE.with(|slot| slot.set(None));
    }

    fn clear_paths() {
        RENDER_PATHS.with(|slot| slot.borrow_mut().clear());
        EFFECT_PATHS.with(|slot| slot.borrow_mut().clear());
    }

    fn put_v2(value: &str) {
        write_v2(Some(value));
    }

    fn put_legacy(value: &str) {
        write_legacy(Some(value));
    }

    fn season_row(id: &str, name: &str, is_current: bool) -> Season {
        use chrono::{TimeZone, Utc};
        Season {
            id: id.to_string(),
            name: name.to_string(),
            starts_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            ends_at: Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap(),
            is_current,
        }
    }

    fn set_seasons(rows: Vec<Season>) {
        TEST_SEASONS.with(|slot| *slot.borrow_mut() = SeasonFetch::Ready(rows));
    }

    fn fail_seasons() {
        TEST_SEASONS.with(|slot| *slot.borrow_mut() = SeasonFetch::Failed);
    }

    fn render_paths() -> Vec<String> {
        RENDER_PATHS.with(|slot| slot.borrow().clone())
    }

    fn effect_paths() -> Vec<String> {
        EFFECT_PATHS.with(|slot| slot.borrow().clone())
    }

    fn season_probe() -> Element {
        let season = use_stats_season();
        TEST_PROBE.with(|slot| slot.set(Some(season)));
        let path = season.fetch_path(ROLE_PATH);
        RENDER_PATHS.with(|slot| slot.borrow_mut().push(path));
        use_effect(move || {
            let path = season.fetch_path(ROLE_PATH);
            EFFECT_PATHS.with(|slot| slot.borrow_mut().push(path));
        });
        rsx! {
            SeasonSelect {
                seasons: season.season_list(),
                seasons_error: season.seasons_error(),
                on_retry: move |_| season.retry(),
                value: season.selected_id(),
                onchange: move |picked| season.choose(picked),
            }
        }
    }

    struct AbortOnTimeout {
        done: Arc<AtomicBool>,
    }

    impl Drop for AbortOnTimeout {
        fn drop(&mut self) {
            self.done.store(true, Ordering::Relaxed);
        }
    }

    fn abort_on_timeout(limit: std::time::Duration) -> AbortOnTimeout {
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            while !flag.load(Ordering::Relaxed) {
                if start.elapsed() > limit {
                    eprintln!("season hook test exceeded {limit:?}");
                    std::process::exit(101);
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        });
        AbortOnTimeout { done }
    }

    fn pump(dom: &mut VirtualDom) {
        for _ in 0..12 {
            dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        }
    }

    fn mount() -> VirtualDom {
        let mut dom = VirtualDom::new(season_probe);
        dom.rebuild_in_place();
        pump(&mut dom);
        dom
    }

    fn choose(dom: &mut VirtualDom, picked: Option<&str>) {
        let picked = picked.map(str::to_string);
        dom.in_runtime(|| {
            let season = TEST_PROBE.with(|slot| slot.get().expect("season probe mounted"));
            season.choose(picked);
        });
        pump(dom);
    }

    fn assert_never_sends_sentinel(paths: &[String]) {
        assert!(
            paths.iter().all(|path| !path.contains("season=current")),
            "season=current must never be requested: {paths:?}"
        );
    }

    fn assert_held_then(paths: &[String], settled: &str) {
        assert_eq!(paths.first().map(String::as_str), Some(""), "{paths:?}");
        assert_eq!(paths.last().map(String::as_str), Some(settled), "{paths:?}");
        assert_never_sends_sentinel(paths);
    }

    #[test]
    fn p_a_saved_current_resolves_to_the_live_id() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        put_v2(CURRENT_SEASON);
        set_seasons(vec![
            season_row("season-4", "Season 4", true),
            season_row("season-3", "Season 3", false),
        ]);
        let dom = mount();
        assert_held_then(&render_paths(), "/api/stats/me/roles?season=season-4");
        assert_eq!(read_v2().as_deref(), Some(CURRENT_SEASON));
        let html = dioxus_ssr::render(&dom);
        assert_option_selected(&html, "current");
        assert_option_not_selected(&html, "season-4");
    }

    #[test]
    fn p_b_nothing_saved_fetches_once() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        set_seasons(vec![season_row("season-4", "Season 4", true)]);
        let _dom = mount();
        let rendered = render_paths();
        assert!(
            rendered.iter().all(|path| path == ROLE_PATH),
            "{rendered:?}"
        );
        let effects = effect_paths();
        assert_eq!(effects, vec![ROLE_PATH.to_string()], "{effects:?}");
    }

    #[test]
    fn p_c_stale_id_is_cleared_after_one_unfiltered_fetch() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        put_v2("season-1");
        set_seasons(vec![season_row("season-4", "Season 4", true)]);
        let _dom = mount();
        assert_held_then(&render_paths(), ROLE_PATH);
        assert_eq!(read_v2(), None);
    }

    #[test]
    fn p_d_failed_list_keeps_storage_and_retry_loads_it() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        put_v2(CURRENT_SEASON);
        fail_seasons();
        let mut dom = mount();
        assert_held_then(&render_paths(), ROLE_PATH);
        assert_eq!(read_v2().as_deref(), Some(CURRENT_SEASON));
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains(">Retry<"), "{html}");
        assert!(!html.contains("Retrying"), "{html}");
        assert!(html.contains("ui-btn--ghost"), "{html}");
        assert!(html.contains("season-select-status"), "{html}");

        set_seasons(vec![season_row("season-4", "Season 4", true)]);
        clear_paths();
        dom.in_runtime(|| {
            let season = TEST_PROBE.with(|slot| slot.get().expect("probe"));
            season.retry();
        });
        pump(&mut dom);
        assert_eq!(
            render_paths().last().map(String::as_str),
            Some("/api/stats/me/roles?season=season-4"),
            "{:?}",
            render_paths()
        );
        assert_never_sends_sentinel(&render_paths());
    }

    #[test]
    fn p_e_sentinel_is_kept_when_no_season_is_current() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        put_v2(CURRENT_SEASON);
        set_seasons(vec![season_row("season-4", "Season 4", false)]);
        let dom = mount();
        assert_held_then(&render_paths(), ROLE_PATH);
        assert_eq!(read_v2().as_deref(), Some(CURRENT_SEASON));
        let html = dioxus_ssr::render(&dom);
        assert_option_selected(&html, "current");
        assert_option_not_selected(&html, "");
        assert!(
            option_chunk(&html, "current").contains(CURRENT_SEASON_GAP_LABEL),
            "{html}"
        );
        drop(dom);

        set_seasons(vec![season_row("season-5", "Season 5", true)]);
        clear_paths();
        let dom = mount();
        assert_held_then(&render_paths(), "/api/stats/me/roles?season=season-5");
        assert_eq!(read_v2().as_deref(), Some(CURRENT_SEASON));
        let html = dioxus_ssr::render(&dom);
        assert_option_selected(&html, "current");
        assert!(
            option_chunk(&html, "current").contains(CURRENT_SEASON_LABEL),
            "{html}"
        );
        assert!(
            !option_chunk(&html, "current").contains("none running"),
            "{html}"
        );
    }

    #[test]
    fn p_f_choosing_the_current_season_follows_the_next_rollover() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        set_seasons(vec![
            season_row("season-4", "Season 4", true),
            season_row("season-3", "Season 3", false),
        ]);
        let mut dom = mount();
        choose(&mut dom, Some(CURRENT_SEASON));
        assert_eq!(read_v2().as_deref(), Some(CURRENT_SEASON));
        assert_eq!(
            render_paths().last().map(String::as_str),
            Some("/api/stats/me/roles?season=season-4")
        );
        drop(dom);

        set_seasons(vec![season_row("season-5", "Season 5", true)]);
        clear_paths();
        let _dom = mount();
        assert_held_then(&render_paths(), "/api/stats/me/roles?season=season-5");
        assert_eq!(read_v2().as_deref(), Some(CURRENT_SEASON));
    }

    #[test]
    fn p_g_past_pin_survives_rollover() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        set_seasons(vec![
            season_row("season-5", "Season 5", true),
            season_row("season-4", "Season 4", false),
        ]);
        let mut dom = mount();
        choose(&mut dom, Some("season-4"));
        assert_eq!(read_v2().as_deref(), Some("season-4"));
        drop(dom);

        set_seasons(vec![
            season_row("season-6", "Season 6", true),
            season_row("season-4", "Season 4", false),
        ]);
        clear_paths();
        let _dom = mount();
        assert_held_then(&render_paths(), "/api/stats/me/roles?season=season-4");
    }

    #[test]
    fn p_h_second_flagged_season_stays_pinned_across_reload() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        set_seasons(vec![
            season_row("season-6", "Season 6", true),
            season_row("season-5", "Season 5", true),
        ]);
        let mut dom = mount();
        choose(&mut dom, Some("season-5"));
        assert_eq!(read_v2().as_deref(), Some("season-5"));
        assert_eq!(
            render_paths().last().map(String::as_str),
            Some("/api/stats/me/roles?season=season-5")
        );
        drop(dom);

        clear_paths();
        let _dom = mount();
        assert_held_then(&render_paths(), "/api/stats/me/roles?season=season-5");
        assert_eq!(read_v2().as_deref(), Some("season-5"));
    }

    #[test]
    fn p_i_legacy_current_id_migrates_and_follows_rollover() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        put_legacy("season-4");
        set_seasons(vec![
            season_row("season-4", "Season 4", true),
            season_row("season-3", "Season 3", false),
        ]);
        let dom = mount();
        assert_held_then(&render_paths(), "/api/stats/me/roles?season=season-4");
        assert_eq!(read_v2().as_deref(), Some(CURRENT_SEASON));
        assert_eq!(read_legacy(), None);
        drop(dom);

        set_seasons(vec![season_row("season-5", "Season 5", true)]);
        clear_paths();
        let _dom = mount();
        assert_held_then(&render_paths(), "/api/stats/me/roles?season=season-5");
    }

    #[test]
    fn p_j_all_time_clears_the_sentinel_and_refetches_once() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        put_v2(CURRENT_SEASON);
        set_seasons(vec![season_row("season-4", "Season 4", true)]);
        let mut dom = mount();
        let before = render_paths().len();
        choose(&mut dom, None);
        assert_eq!(read_v2(), None);
        let after = render_paths();
        assert_eq!(
            after.last().map(String::as_str),
            Some(ROLE_PATH),
            "{after:?}"
        );
        let extra = &after[before..];
        assert!(
            extra.iter().all(|path| path == ROLE_PATH),
            "all time adds one unfiltered request: {extra:?}"
        );
        assert_never_sends_sentinel(&after);
    }

    #[test]
    fn choose_trims_the_in_session_pick() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        set_seasons(vec![
            season_row("season-5", "Season 5", true),
            season_row("season-3", "Season 3", false),
        ]);
        let mut dom = mount();
        choose(&mut dom, Some("  season-3  "));
        assert_eq!(read_v2().as_deref(), Some("season-3"));
        assert_eq!(
            render_paths().last().map(String::as_str),
            Some("/api/stats/me/roles?season=season-3")
        );
        drop(dom);
    }

    #[test]
    fn p_k_upcoming_pin_stays_pinned_after_later_rollovers() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        set_seasons(vec![
            season_row("season-5", "Season 5", false),
            season_row("season-4", "Season 4", true),
        ]);
        let mut dom = mount();
        choose(&mut dom, Some("season-5"));
        assert_eq!(read_v2().as_deref(), Some("season-5"));
        drop(dom);

        set_seasons(vec![
            season_row("season-6", "Season 6", true),
            season_row("season-5", "Season 5", false),
        ]);
        clear_paths();
        let _dom = mount();
        assert_held_then(&render_paths(), "/api/stats/me/roles?season=season-5");
    }

    #[test]
    fn p_l_empty_list_keeps_the_sentinel() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        put_v2(CURRENT_SEASON);
        set_seasons(vec![]);
        let _dom = mount();
        assert_held_then(&render_paths(), ROLE_PATH);
        assert_eq!(read_v2().as_deref(), Some(CURRENT_SEASON));
    }

    fn option_chunk<'a>(html: &'a str, value: &str) -> &'a str {
        html.split("<option")
            .skip(1)
            .find(|part| part.contains(&format!("value=\"{value}\"")))
            .unwrap_or_else(|| panic!("missing option {value} in {html}"))
    }

    fn assert_option_selected(html: &str, value: &str) {
        let chunk = option_chunk(html, value);
        assert!(
            chunk.contains("selected=true"),
            "option {value} should be selected: {html}"
        );
    }

    fn assert_option_not_selected(html: &str, value: &str) {
        let chunk = option_chunk(html, value);
        assert!(
            !chunk.contains("selected=true"),
            "option {value} should not be selected: {html}"
        );
    }

    fn is_selected(edit: &Mutation, want: bool) -> bool {
        matches!(
            edit,
            Mutation::SetAttribute {
                name: "selected",
                value: AttributeValue::Bool(value),
                ..
            } if *value == want
        )
    }

    fn brief(edits: &[Mutation]) -> String {
        edits
            .iter()
            .enumerate()
            .map(|(i, edit)| match edit {
                Mutation::SetAttribute {
                    name, value, id, ..
                } => {
                    format!("{i} set {name}={value:?} id={id:?}")
                }
                Mutation::CreateTextNode { value, id } => format!("{i} text {value:?} id={id:?}"),
                Mutation::AssignId { path, id } => format!("{i} assign {path:?} id={id:?}"),
                Mutation::InsertBefore { m, .. } => format!("{i} insert_before m={m}"),
                Mutation::InsertAfter { m, .. } => format!("{i} insert_after m={m}"),
                Mutation::AppendChildren { m, .. } => format!("{i} append m={m}"),
                Mutation::ReplacePlaceholder { m, .. } => format!("{i} replace_ph m={m}"),
                Mutation::ReplaceWith { m, .. } => format!("{i} replace m={m}"),
                Mutation::LoadTemplate { .. } => format!("{i} template"),
                _ => format!("{i} other"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn pinned_option_is_marked_selected_before_it_is_inserted() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        put_v2("season-4");
        set_seasons(vec![
            season_row("season-5", "Season 5", true),
            season_row("season-4", "Season 4", false),
        ]);
        let mut dom = VirtualDom::new(season_probe);
        let mut edits = Mutations::default();
        dom.rebuild(&mut edits);
        let mut all = edits.edits;
        for _ in 0..12 {
            let mut batch = Mutations::default();
            dom.render_immediate(&mut batch);
            all.extend(batch.edits);
        }
        let log = brief(&all);
        let select_id = all
            .iter()
            .find_map(|edit| match edit {
                Mutation::SetAttribute {
                    name: "value",
                    value: AttributeValue::Text(text),
                    id,
                    ..
                } if text == "season-4" => Some(*id),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing select value\n{log}"));
        let select_path = all
            .iter()
            .find_map(|edit| match edit {
                Mutation::AssignId { path, id } if *id == select_id => Some(*path),
                _ => None,
            })
            .unwrap_or_else(|| panic!("select was not assigned an id\n{log}"));
        let all_time_id = all
            .iter()
            .find_map(|edit| match edit {
                Mutation::AssignId { path, id }
                    if path.len() == select_path.len() + 1
                        && path[..select_path.len()] == *select_path
                        && path[select_path.len()] == 0 =>
                {
                    Some(*id)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing All time option\n{log}"));
        let season_count = 2;
        let insert_at = all
            .iter()
            .position(
                |edit| matches!(edit, Mutation::ReplacePlaceholder { m, .. } if *m == season_count),
            )
            .unwrap_or_else(|| panic!("season options were not inserted\n{log}"));
        let all_time_cleared = all[..insert_at].iter().any(|edit| {
            matches!(
                edit,
                Mutation::SetAttribute {
                    name: "selected",
                    value: AttributeValue::Bool(false),
                    id,
                    ..
                } if *id == all_time_id
            )
        });
        assert!(
            all_time_cleared,
            "All time must be explicitly unselected before the options are inserted\n{log}"
        );
        assert!(
            all[..insert_at].iter().any(|edit| is_selected(edit, true)),
            "selected=true must be set before the options are inserted\n{log}"
        );
        let html = dioxus_ssr::render(&dom);
        assert_option_selected(&html, "season-4");
        assert_option_not_selected(&html, "current");
        assert_option_not_selected(&html, "season-5");
        assert!(
            render_paths()
                .last()
                .is_some_and(|path| path.ends_with("season=season-4")),
            "{:?}",
            render_paths()
        );
    }

    #[test]
    fn explicit_current_option_stores_the_sentinel() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        set_seasons(vec![
            season_row("season-5", "Season 5", false),
            season_row("season-4", "Season 4", true),
        ]);
        let mut dom = mount();
        choose(&mut dom, Some(CURRENT_SEASON));
        assert_eq!(read_v2().as_deref(), Some(CURRENT_SEASON));
        assert_eq!(
            render_paths().last().map(String::as_str),
            Some("/api/stats/me/roles?season=season-4")
        );
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains("Current season"), "{html}");
        assert_eq!(html.matches("selected=true").count(), 1, "{html}");
    }

    #[test]
    fn choosing_the_current_row_pins_it() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        set_seasons(vec![
            season_row("season-4", "Season 4", true),
            season_row("season-3", "Season 3", false),
        ]);
        let mut dom = mount();
        choose(&mut dom, Some("season-4"));
        assert_eq!(read_v2().as_deref(), Some("season-4"));
        let html = dioxus_ssr::render(&dom);
        assert_option_selected(&html, "season-4");
        assert_option_not_selected(&html, "current");

        choose(&mut dom, Some(CURRENT_SEASON));
        assert_eq!(read_v2().as_deref(), Some(CURRENT_SEASON));
        let html = dioxus_ssr::render(&dom);
        assert_option_selected(&html, "current");
        assert_option_not_selected(&html, "season-4");

        choose(&mut dom, Some("season-4"));
        assert_eq!(read_v2().as_deref(), Some("season-4"));
        let html = dioxus_ssr::render(&dom);
        assert_option_selected(&html, "season-4");
        assert_option_not_selected(&html, "current");
        assert!(
            render_paths()
                .last()
                .is_some_and(|path| path.ends_with("season=season-4")),
            "{:?}",
            render_paths()
        );
    }

    #[test]
    fn p2_12_legacy_current_id_selects_current_on_the_migrating_mount() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        put_legacy("season-4");
        set_seasons(vec![
            season_row("season-4", "Season 4", true),
            season_row("season-3", "Season 3", false),
        ]);
        let dom = mount();
        let html = dioxus_ssr::render(&dom);
        assert_option_selected(&html, "current");
        assert_option_not_selected(&html, "season-4");
        assert!(
            render_paths()
                .last()
                .is_some_and(|path| path.ends_with("?season=season-4")),
            "{:?}",
            render_paths()
        );
    }

    #[test]
    fn p2_13_legacy_key_survives_loading_and_failure_then_retry_migrates_it() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        put_legacy("season-4");
        let mut dom = mount();
        assert_eq!(read_legacy().as_deref(), Some("season-4"));
        assert_eq!(read_v2(), None);
        assert!(
            render_paths().iter().all(|path| path.is_empty()),
            "{:?}",
            render_paths()
        );

        fail_seasons();
        dom.in_runtime(|| {
            let season = TEST_PROBE.with(|slot| slot.get().expect("probe"));
            season.retry();
        });
        pump(&mut dom);
        assert_eq!(read_legacy().as_deref(), Some("season-4"));
        assert_eq!(read_v2(), None);
        assert_eq!(render_paths().last().map(String::as_str), Some(ROLE_PATH));

        set_seasons(vec![season_row("season-4", "Season 4", true)]);
        clear_paths();
        dom.in_runtime(|| {
            let season = TEST_PROBE.with(|slot| slot.get().expect("probe"));
            season.retry();
        });
        pump(&mut dom);
        assert_eq!(read_v2().as_deref(), Some(CURRENT_SEASON));
        assert_eq!(read_legacy(), None);
        assert!(
            render_paths()
                .last()
                .is_some_and(|path| path.ends_with("?season=season-4")),
            "{:?}",
            render_paths()
        );
    }

    #[test]
    fn both_keys_present_keeps_v2_and_drops_legacy() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        put_v2("season-3");
        put_legacy("season-4");
        set_seasons(vec![
            season_row("season-4", "Season 4", true),
            season_row("season-3", "Season 3", false),
        ]);
        let dom = mount();
        assert_eq!(read_v2().as_deref(), Some("season-3"));
        assert_eq!(read_legacy(), None);
        assert!(
            render_paths()
                .last()
                .is_some_and(|path| path.ends_with("?season=season-3")),
            "{:?}",
            render_paths()
        );
        let html = dioxus_ssr::render(&dom);
        assert_option_selected(&html, "season-3");
        assert_option_not_selected(&html, "season-4");
    }

    #[test]
    fn retry_stays_mounted_while_the_next_fetch_is_in_flight() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        fail_seasons();
        let mut dom = mount();
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains(">Retry<"), "{html}");
        assert!(html.contains("ui-btn--ghost"), "{html}");
        assert!(html.contains("ui-btn--sm"), "{html}");
        assert!(!html.contains("Retrying"), "{html}");

        TEST_SEASONS.with(|slot| *slot.borrow_mut() = SeasonFetch::Hold);
        dom.in_runtime(|| {
            let season = TEST_PROBE.with(|slot| slot.get().expect("probe"));
            season.retry();
        });
        pump(&mut dom);
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains("Retrying..."), "{html}");
        assert!(html.contains("disabled=true"), "{html}");
        assert!(html.contains("ui-btn--ghost"), "{html}");
        assert!(html.contains("season-select-status"), "{html}");
    }

    #[test]
    fn stats_pages_route_season_fetches_through_the_filter() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for rel in [
            "pages/stats/mod.rs",
            "pages/stats_member.rs",
            "pages/leaderboards.rs",
        ] {
            let text = std::fs::read_to_string(root.join(rel)).unwrap_or_else(|err| {
                panic!("read {rel}: {err}");
            });
            assert!(
                !text.contains("season_url("),
                "{rel} must not call season_url"
            );
        }
        let stats = std::fs::read_to_string(root.join("pages/stats/mod.rs")).unwrap();
        let member = std::fs::read_to_string(root.join("pages/stats_member.rs")).unwrap();
        let boards = std::fs::read_to_string(root.join("pages/leaderboards.rs")).unwrap();
        for (rel, text) in [
            ("pages/stats/mod.rs", &stats),
            ("pages/stats_member.rs", &member),
        ] {
            let fetches = text.matches("use_api_with::<").count();
            let filtered = text.matches("season.fetch_path").count();
            assert!(
                fetches > 0 && filtered >= fetches,
                "{rel} has {fetches} use_api_with and {filtered} season.fetch_path"
            );
        }
        assert!(
            stats.contains("season.fetch_path(role::my_roles_path())"),
            "my stats role request must use the resolved filter"
        );
        assert!(
            member.contains("season.fetch_path(&member_roles_path("),
            "member role request must use the resolved filter"
        );
        assert!(boards.contains("season.fetch_path("));
        assert!(boards.contains("leaderboard_hold("));
        assert!(boards.contains("return held;"));
        assert!(
            boards.contains("leaderboard_message(snapshot.as_ref()"),
            "leaderboards must render through leaderboard_message"
        );
        let picker = std::fs::read_to_string(root.join("components/ui/season_select.rs"))
            .expect("season select source");
        // Split so this assertion does not contain the call it is looking for.
        let retry_call = format!("on_retry.{}", "call(())");
        assert!(
            picker.contains(&retry_call),
            "the Retry button must call on_retry"
        );
        assert!(
            picker.contains("value: \"current\""),
            "the Current season option value is the literal sentinel"
        );
        for (rel, text) in [
            ("pages/stats/mod.rs", stats.as_str()),
            ("pages/stats_member.rs", member.as_str()),
            ("pages/leaderboards.rs", boards.as_str()),
        ] {
            assert!(
                text.contains("season.retry()"),
                "{rel} must retry the season list"
            );
        }
    }

    fn labeled_season_probe() -> Element {
        let season = use_stats_season();
        rsx! {
            SeasonSelect {
                id: "leaderboard-season".to_string(),
                name: "leaderboard-season".to_string(),
                label: "Season".to_string(),
                seasons: season.season_list(),
                seasons_error: season.seasons_error(),
                on_retry: move |_| season.retry(),
                value: season.selected_id(),
                onchange: move |picked| season.choose(picked),
            }
        }
    }

    #[test]
    fn labeled_season_select_sets_id_name_and_for() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        set_seasons(vec![season_row("season-4", "Season 4", true)]);
        let mut dom = VirtualDom::new(labeled_season_probe);
        dom.rebuild_in_place();
        pump(&mut dom);
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains("id=\"leaderboard-season\""), "{html}");
        assert!(html.contains("name=\"leaderboard-season\""), "{html}");
        assert!(html.contains("for=\"leaderboard-season\""), "{html}");
        assert!(html.contains(">Season<"), "{html}");
        assert!(
            !html.contains("aria-label"),
            "a wired label should be the accessible name: {html}"
        );
    }

    #[test]
    fn failed_season_label_does_not_point_at_a_missing_select() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        fail_seasons();
        let mut dom = VirtualDom::new(labeled_season_probe);
        dom.rebuild_in_place();
        pump(&mut dom);
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains(">Season<"), "{html}");
        assert!(html.contains("season-select-status"), "{html}");
        assert!(html.contains("load seasons."), "{html}");
        assert!(!html.contains("<select"), "{html}");
        assert!(!html.contains("<label"), "{html}");
        assert!(!html.contains("for="), "{html}");
    }

    #[test]
    fn loading_season_label_has_no_for() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        let mut dom = VirtualDom::new(labeled_season_probe);
        dom.rebuild_in_place();
        pump(&mut dom);
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains(">Season<"), "{html}");
        assert!(!html.contains("<select"), "{html}");
        assert!(!html.contains("<label"), "{html}");
        assert!(!html.contains("for="), "{html}");
    }

    #[test]
    fn storage_event_only_follows_the_live_season_key() {
        assert!(storage_event_targets_season(Some(STATS_SEASON_KEY)));
        assert!(storage_event_targets_season(None));
        assert!(!storage_event_targets_season(Some(LEGACY_SEASON_KEY)));
        assert!(!storage_event_targets_season(Some("stats-ui-density")));
        assert!(!storage_event_targets_season(Some("")));
    }

    fn fire_storage(dom: &mut VirtualDom, key: Option<&str>) {
        dom.in_runtime(|| {
            let season = TEST_PROBE.with(|slot| slot.get().expect("season probe mounted"));
            season.apply_storage_event(key);
        });
        pump(dom);
    }

    #[test]
    fn other_tab_season_write_replaces_the_in_session_pick() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        set_seasons(vec![
            season_row("season-4", "Season 4", true),
            season_row("season-3", "Season 3", false),
        ]);
        let mut dom = mount();
        choose(&mut dom, Some("season-3"));
        assert_eq!(read_v2().as_deref(), Some("season-3"));

        // Storage moved, but an unrelated key must not drop this tab's pick.
        put_v2(CURRENT_SEASON);
        fire_storage(&mut dom, Some("stats-ui-density"));
        assert_eq!(read_v2().as_deref(), Some(CURRENT_SEASON));
        let html = dioxus_ssr::render(&dom);
        assert_option_selected(&html, "season-3");
        assert!(
            render_paths()
                .last()
                .is_some_and(|path| path.ends_with("season=season-3")),
            "{:?}",
            render_paths()
        );

        fire_storage(&mut dom, Some(STATS_SEASON_KEY));
        assert_eq!(read_v2().as_deref(), Some(CURRENT_SEASON));
        let html = dioxus_ssr::render(&dom);
        assert_option_selected(&html, "current");
        assert_option_not_selected(&html, "season-3");
        assert!(
            render_paths()
                .last()
                .is_some_and(|path| path.ends_with("season=season-4")),
            "{:?}",
            render_paths()
        );
    }

    #[test]
    fn cleared_storage_event_returns_to_all_time() {
        let _timeout = abort_on_timeout(std::time::Duration::from_secs(8));
        blank_hooks();
        set_seasons(vec![season_row("season-4", "Season 4", true)]);
        let mut dom = mount();
        choose(&mut dom, Some(CURRENT_SEASON));
        assert_eq!(read_v2().as_deref(), Some(CURRENT_SEASON));

        write_v2(None);
        fire_storage(&mut dom, None);
        assert_eq!(read_v2(), None);
        let html = dioxus_ssr::render(&dom);
        assert_option_selected(&html, "");
        assert_option_not_selected(&html, "current");
        assert_eq!(render_paths().last().map(String::as_str), Some(ROLE_PATH));
    }

    #[test]
    fn season_storage_listener_is_installed_and_removed() {
        let src = include_str!("season_select.rs");
        let add = format!("add_event_listener_with_callback({}", "\"storage\"");
        let remove = format!("remove_event_listener_with_callback({}", "\"storage\"");
        assert!(src.contains(&add), "storage listener must be registered");
        assert!(
            src.contains(&remove),
            "storage listener must be removed on unmount"
        );
        let drop_call = format!("use_{}(move", "drop");
        assert!(src.contains(&drop_call), "removal belongs in use_drop");
        let hook = format!("use_season_storage_sync{}", "(");
        assert!(
            src.matches(&hook).count() >= 3,
            "wasm, native stub, and the hook call must all exist"
        );
        let note = format!("note_season_storage{}", "(");
        assert!(
            src.matches(&note).count() >= 3,
            "the listener and the test hook must share the re-read"
        );
    }
}
