//! Season picker shared by every stats surface: "All time" plus each season
//! from `GET /api/public/seasons` (the current season is marked). Renders
//! nothing while no seasons exist — there is nothing to switch between.
//!
//! Picking the season flagged `is_current` stores the sentinel [`CURRENT_SEASON`]
//! instead of that season's id, so a later rollover follows the new current
//! season. A past season is stored as its id. A saved value that the loaded
//! list cannot resolve becomes all time and is removed. Nothing is saved
//! means all time, including before the list arrives.

use dioxus::prelude::*;
use scuffed_types::Season;

use crate::components::ui::Label;
use crate::hooks::use_api;
use crate::util::season_url;

pub const ALL_TIME_LABEL: &str = "All time";

/// `stats-season` value meaning "whichever season is current right now".
const CURRENT_SEASON: &str = "current";

const STATS_SEASON_KEY: &str = "stats-season";

pub const SEASON_SELECT_CSS: &str = r#"
.season-select { display: flex; flex-direction: column; gap: var(--space-1); }
"#;

/// One season the picker can resolve a saved value against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeasonChoice<'a> {
    pub id: &'a str,
    pub is_current: bool,
}

/// Resolve a stored `stats-season` value against the loaded season list.
///
/// - missing or blank storage → `None` (all time)
/// - [`CURRENT_SEASON`] → the id with `is_current`, or `None` when none is current
/// - any other string → that id when it is in `seasons`, otherwise `None`
pub fn resolve_stored_season<'a>(
    stored: Option<&str>,
    seasons: &[SeasonChoice<'a>],
) -> Option<&'a str> {
    let stored = stored.map(str::trim).filter(|s| !s.is_empty())?;
    if stored == CURRENT_SEASON {
        return seasons.iter().find(|s| s.is_current).map(|s| s.id);
    }
    seasons.iter().find(|s| s.id == stored).map(|s| s.id)
}

/// What to write for a picker choice. `None` clears storage (all time).
/// The current season is stored as [`CURRENT_SEASON`]; any other id is pinned.
pub fn season_to_store<'a>(
    selected: Option<&'a str>,
    seasons: &[SeasonChoice<'_>],
) -> Option<&'a str> {
    let selected = selected.map(str::trim).filter(|s| !s.is_empty())?;
    if seasons.iter().any(|s| s.is_current && s.id == selected) {
        Some(CURRENT_SEASON)
    } else {
        Some(selected)
    }
}

/// Saved value that the loaded list cannot turn into a season id.
fn stored_season_is_stale(stored: Option<&str>, seasons: &[SeasonChoice<'_>]) -> bool {
    match stored.map(str::trim).filter(|s| !s.is_empty()) {
        None => false,
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
            if stored.map(str::trim).is_some_and(|s| !s.is_empty()) {
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

/// Request path for a resolved filter. Pending yields an empty path so callers
/// skip the fetch instead of sending `season=current` or an unchecked id.
fn season_filter_path(resolved: &ResolvedSeason, path: &str) -> String {
    match resolved {
        ResolvedSeason::Pending => String::new(),
        ResolvedSeason::All => path.to_string(),
        ResolvedSeason::Id(id) => season_url(path, Some(id.clone())),
    }
}

#[derive(Clone, PartialEq, Eq)]
enum UserSeasonPick {
    All,
    Id(String),
}

/// Shared stats-season filter. Copy it into each fetch closure.
///
/// [`Self::fetch_path`] reads only the resolved memo. Reading
/// [`Self::season_list`] inside a fetch closure subscribes that fetch to the
/// season list and refetches whenever the list updates.
#[derive(Clone, Copy)]
pub struct StatsSeason {
    resolved: Memo<ResolvedSeason>,
    /// `Resource` is `Copy` without requiring `Season: Copy`. `ApiResource` is
    /// not, because its derive adds that bound.
    rows: Resource<Option<Vec<Season>>>,
    pick: Signal<Option<UserSeasonPick>>,
}

impl StatsSeason {
    fn current(self) -> ResolvedSeason {
        (self.resolved)()
    }

    /// Season id for the `<select>`, or `None` for all time.
    pub fn selected_id(self) -> Option<String> {
        match self.current() {
            ResolvedSeason::Id(id) => Some(id),
            ResolvedSeason::All | ResolvedSeason::Pending => None,
        }
    }

    /// Handle for the shared `/api/public/seasons` fetch. The picker reads it;
    /// stats requests must use [`Self::fetch_path`] so they do not subscribe
    /// to the list.
    pub fn season_list(self) -> Resource<Option<Vec<Season>>> {
        self.rows
    }

    /// Path to fetch, or `""` while a saved season is still unresolved.
    /// `use_api_with` treats an empty path as "do not send".
    pub fn fetch_path(self, path: &str) -> String {
        season_filter_path(&self.current(), path)
    }

    /// Record a picker change. `None` is all time. The current season is
    /// stored as the sentinel; a past season id is pinned.
    pub fn choose(mut self, picked: Option<String>) {
        let data = self.rows.read();
        let choices = match data.as_ref().and_then(|inner| inner.as_ref()) {
            Some(list) => choices_from(list),
            None => Vec::new(),
        };
        write_stats_season(season_to_store(picked.as_deref(), &choices));
        let next = match picked {
            Some(id) => Some(UserSeasonPick::Id(id)),
            None => Some(UserSeasonPick::All),
        };
        if (self.pick)() != next {
            self.pick.set(next);
        }
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

/// Last chosen season (per browser). Blank storage is all time.
fn read_stats_season() -> Option<String> {
    #[cfg(feature = "web")]
    {
        if let Some(win) = web_sys::window()
            && let Ok(Some(storage)) = win.local_storage()
            && let Ok(Some(v)) = storage.get_item(STATS_SEASON_KEY)
        {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn write_stats_season(value: Option<&str>) {
    #[cfg(feature = "web")]
    {
        if let Some(win) = web_sys::window()
            && let Ok(Some(storage)) = win.local_storage()
        {
            let _ = match value.map(str::trim).filter(|s| !s.is_empty()) {
                Some(v) => storage.set_item(STATS_SEASON_KEY, v),
                None => storage.remove_item(STATS_SEASON_KEY),
            };
        }
    }
    #[cfg(not(feature = "web"))]
    let _ = value;
}

pub fn use_stats_season() -> StatsSeason {
    let pick = use_signal(|| None::<UserSeasonPick>);
    let rows = use_api::<Vec<Season>>("/api/public/seasons").data;
    // The memo is the only signal stats resources should read. It notifies
    // only when PartialEq says the filter changed, so "nothing saved" stays
    // one all-time fetch after the season list arrives.
    let resolved = use_memo(move || {
        if let Some(choice) = pick() {
            return match choice {
                UserSeasonPick::All => ResolvedSeason::All,
                UserSeasonPick::Id(id) => ResolvedSeason::Id(id),
            };
        }
        let stored = read_stats_season();
        let data = rows.read();
        match data.as_ref() {
            None => resolve_season_filter(stored.as_deref(), SeasonList::Loading),
            Some(None) => resolve_season_filter(stored.as_deref(), SeasonList::Unavailable),
            Some(Some(list)) => {
                let choices = choices_from(list);
                resolve_season_filter(stored.as_deref(), SeasonList::Ready(&choices))
            }
        }
    });

    // Drop a saved value the loaded list cannot resolve. localStorage is not
    // a signal; writing `pick` here would make the memo and the stats
    // resources chase each other.
    use_effect(move || {
        if pick().is_some() {
            return;
        }
        let data = rows.read();
        let Some(list) = data.as_ref().and_then(|inner| inner.as_ref()) else {
            return;
        };
        let choices = choices_from(list);
        if stored_season_is_stale(read_stats_season().as_deref(), &choices) {
            write_stats_season(None);
        }
    });

    StatsSeason {
        resolved,
        rows,
        pick,
    }
}

#[component]
pub fn SeasonSelect(
    /// Selected season id; `None` = all time.
    value: Option<String>,
    /// Fired on change. `None` = all time; `Some(id)` = that season's id.
    onchange: EventHandler<Option<String>>,
    /// Shared seasons fetch. The control subscribes by reading it.
    seasons: Resource<Option<Vec<Season>>>,
    /// Optional `id` on the underlying `<select>`.
    #[props(default)]
    id: Option<String>,
    /// Optional visible label rendered above the control.
    #[props(default)]
    label: Option<String>,
) -> Element {
    let current = value.unwrap_or_default();
    let data = seasons.read();
    let Some(list) = data.as_ref().and_then(|inner| inner.as_ref()) else {
        return rsx! {};
    };
    if list.is_empty() {
        return rsx! {};
    }
    rsx! {
        div { class: "season-select",
            if let Some(label) = label {
                Label { {label} }
            }
            select {
                class: "ui-field",
                id,
                value: "{current}",
                "aria-label": "Season",
                onchange: move |e| {
                    let v = e.value();
                    onchange.call(if v.is_empty() { None } else { Some(v) });
                },
                option { key: "all", value: "", "{ALL_TIME_LABEL}" }
                for s in list.iter() {
                    option {
                        key: "{s.id}",
                        value: "{s.id}",
                        if s.is_current { "{s.name} (current)" } else { "{s.name}" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn picking_the_current_season_stores_the_sentinel() {
        let stored = season_to_store(Some("season-4"), &during_season_four());
        assert_eq!(stored, Some(CURRENT_SEASON));
        let after = after_rollover();
        assert_eq!(resolve_stored_season(stored, &after), Some("season-5"));
    }

    #[test]
    fn picking_a_past_season_stores_that_id() {
        assert_eq!(
            season_to_store(Some("season-4"), &after_rollover()),
            Some("season-4")
        );
    }

    #[test]
    fn picking_all_time_clears_storage() {
        assert_eq!(season_to_store(None, &after_rollover()), None);
        assert_eq!(season_to_store(Some("  "), &after_rollover()), None);
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
    fn stale_storage_is_only_an_unresolved_value() {
        let seasons = after_rollover();
        assert!(stored_season_is_stale(Some("missing"), &seasons));
        assert!(stored_season_is_stale(Some(CURRENT_SEASON), &[]));
        assert!(!stored_season_is_stale(None, &seasons));
        assert!(!stored_season_is_stale(Some(""), &seasons));
        assert!(!stored_season_is_stale(Some(CURRENT_SEASON), &seasons));
        assert!(!stored_season_is_stale(Some("season-4"), &seasons));
    }
}
