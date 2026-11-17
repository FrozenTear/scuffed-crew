use dioxus::prelude::*;

use serde::Deserialize;

use super::stats::role::{
    RolePanel, hide_member_role_error, member_roles_path, role_aggs_from_rows, role_panel,
    role_result,
};
use super::{NotFoundLink, NotFoundPanel};
use crate::components::member_gate::Wall;
use crate::components::ui::{
    EmptyCard, LoadError, PageHeader, PageShell, PageWidth, SeasonSelect, SkeletonBars, TabButton,
    btn_class, use_stats_season,
};
use crate::components::{is_http_status, member_pending};
use crate::hooks::use_api_with;
use crate::routes::Route;
use crate::sample::{
    HERO_RULE_NOTE, MAP_RULE_NOTE, OTHER_HEROES, OTHER_RESULT_CLASS, Tally, fold_small, other_label,
};
use crate::state::use_auth;
use crate::stats_privacy;

#[derive(Debug, Clone, Deserialize)]
struct PersonalStats {
    #[allow(dead_code)]
    member_id: String,
    total_matches: u32,
    wins: u32,
    losses: u32,
    draws: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct HeroStats {
    hero: String,
    matches: u32,
    wins: u32,
    losses: u32,
    draws: u32,
    avg_elims: f64,
    avg_deaths: f64,
    avg_damage: f64,
    avg_healing: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct MapStats {
    map_name: String,
    matches: u32,
    wins: u32,
    losses: u32,
    draws: u32,
}

impl HeroStats {
    fn tally(&self) -> Tally {
        Tally::new(self.matches, self.wins, self.losses, self.draws)
    }
}

impl MapStats {
    fn tally(&self) -> Tally {
        Tally::new(self.matches, self.wins, self.losses, self.draws)
    }
}

/// Text class for a hero or map result: muted until it has a win rate.
pub(crate) fn result_class(tally: Tally) -> &'static str {
    if tally.has_win_rate() {
        "stats-wr"
    } else {
        "stats-wr muted"
    }
}

#[derive(Clone, Copy, PartialEq)]
enum MemberStatsTab {
    Overview,
    Heroes,
    Maps,
}

/// The one tabpanel under the member stats tabs.
const MEMBER_STATS_PANEL: &str = "member-stats-panel";

/// Id of a member stats tab button. The tabpanel is labelled by the selected one.
fn member_tab_id(tab: MemberStatsTab) -> &'static str {
    match tab {
        MemberStatsTab::Overview => "member-stats-tab-overview",
        MemberStatsTab::Heroes => "member-stats-tab-heroes",
        MemberStatsTab::Maps => "member-stats-tab-maps",
    }
}

/// Style id of [`STATS_VIEWS_CSS`].
pub(crate) const STATS_VIEWS_CSS_ID: &str = "scuffed-css-pages-stats-views-css";

/// Pieces the stats detail views share: the Heroes, Maps and History tabs of
/// `/stats`, `/stats/member/:id`, `/stats/match/:id` and `/stats/tokens`.
/// It lives here because this module is the one all of them can reach.
/// Tokens only, so `data-density` and the theme apply.
pub(crate) const STATS_VIEWS_CSS: &str = r#"
.stats-view-body {
  display: flex;
  flex-direction: column;
  gap: var(--space-section);
  min-width: 0;
}
.stats-view-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  gap: var(--space-3) var(--space-5);
  min-width: 0;
}
.stats-view-filter {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  min-width: 0;
  max-width: 100%;
}
.stats-view-filter-label {
  font-family: var(--font-head);
  font-size: var(--text-xs);
  font-weight: var(--font-label-weight);
  letter-spacing: var(--font-head-tracking);
  text-transform: uppercase;
  color: var(--text-subtle);
}
.stats-view-note {
  margin: 0;
  max-width: var(--content-max-narrow);
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-body);
}
.stats-view-card {
  box-sizing: border-box;
  min-width: 0;
  padding: var(--card-padding);
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
}
.stats-view-section-head {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-1) var(--space-4);
  min-width: 0;
}
.stats-view-section-head .stats-view-note { max-width: none; }

/* Tiles: the value over its label, centred, as on the stats frame. */
.stats-view-tiles {
  display: grid;
  grid-auto-flow: column;
  grid-auto-columns: minmax(0, 1fr);
  gap: var(--space-3);
  margin: 0;
  min-width: 0;
}
.stats-view-tile {
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-1);
  min-width: 0;
  margin: 0;
  padding: var(--card-padding);
  background: var(--card-bg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
  text-align: center;
}
.stats-view-tile-value {
  max-width: 100%;
  margin: 0;
  font-family: var(--font-head);
  font-weight: var(--font-head-weight);
  font-size: var(--text-2xl);
  letter-spacing: var(--font-head-tracking);
  line-height: var(--leading-display);
  font-variant-numeric: tabular-nums;
  color: var(--text);
  overflow-wrap: anywhere;
}
.stats-view-tile-label {
  margin: 0;
  color: var(--text-3);
  font-size: var(--text-sm);
  line-height: var(--leading-ui);
}

/* Tables: the leaderboards recipe on the --table-* tokens. The wrap is the
   scroll container: a named, focusable region, so a wide table scrolls inside
   it at phone width and the page never scrolls sideways. */
.stats-table-block {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
}
.stats-table-hint {
  display: none;
  margin: 0;
  color: var(--text-3);
  font-size: var(--text-xs);
  line-height: var(--leading-ui);
}
.stats-table-wrap {
  box-sizing: border-box;
  width: 100%;
  max-width: 100%;
  overflow-x: auto;
  background: var(--table-row-bg);
  border: 1px solid var(--card-border);
  border-radius: var(--table-radius);
  box-shadow: var(--card-shadow);
}
.stats-table-wrap:focus-visible {
  outline: var(--focus-ring-width) solid var(--focus-ring);
  outline-offset: var(--focus-ring-offset);
}
.stats-table {
  width: 100%;
  border-collapse: collapse;
  font-family: var(--table-font);
  font-size: var(--table-font-size);
  color: var(--table-row-fg);
}
.stats-table thead th {
  box-sizing: border-box;
  height: var(--table-row-height);
  padding: 0 var(--table-pad-x);
  border-bottom: 1px solid var(--table-row-border);
  background: var(--table-row-bg);
  color: var(--table-header-fg);
  font-family: var(--font-head);
  font-size: var(--text-xs);
  font-weight: var(--table-header-weight);
  letter-spacing: var(--font-head-tracking);
  text-transform: uppercase;
  text-align: left;
  white-space: nowrap;
}
.stats-table tbody th,
.stats-table tbody td {
  box-sizing: border-box;
  height: var(--table-row-height);
  padding: var(--space-1) var(--table-pad-x);
  border-bottom: 1px solid var(--table-row-border);
  background: var(--table-row-bg);
  color: var(--table-row-fg);
  text-align: left;
}
.stats-table tbody tr:last-child th,
.stats-table tbody tr:last-child td { border-bottom: 0; }
.stats-table tbody th {
  font-weight: var(--font-label-weight);
  color: var(--text);
  overflow-wrap: anywhere;
}
.stats-table thead th.num,
.stats-table thead th.rate,
.stats-table tbody td.num,
.stats-table tbody td.rate {
  text-align: right;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.stats-table tbody td.num { color: var(--table-row-muted); }
.stats-table .rate { width: calc(var(--space-8) * 5); }
.stats-rate {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  justify-content: center;
  gap: var(--space-1);
}
/* The bar repeats the number next to it. Its fill is neutral ink, so a rate
   is never read from a colour. */
.stats-rate-bar {
  display: block;
  width: 100%;
  height: var(--space-1);
  background: var(--border);
  border-radius: var(--radius-pill);
  overflow: hidden;
}
.stats-rate-fill {
  display: block;
  height: 100%;
  background: var(--text-3);
  border-radius: var(--radius-pill);
}
.stats-table td .stats-wr {
  display: inline;
  font-size: inherit;
  font-weight: var(--font-label-weight);
  color: var(--table-row-fg);
  font-variant-numeric: tabular-nums;
}
.stats-table td .stats-wr.muted {
  font-weight: 400;
  color: var(--text-subtle);
}
/* A small sample is quieter through its ink, never through opacity, so the
   text keeps its contrast. */
.stats-table tr.stats-row-muted { opacity: 1; }
.stats-table tr.stats-row-muted th,
.stats-table tr.stats-row-muted td {
  color: var(--text-subtle);
  font-weight: 400;
}
.stats-table .stats-table-actions { text-align: right; white-space: nowrap; }

@media (max-width: 720px) {
  .stats-table-block.is-wide .stats-table-hint { display: block; }
  .stats-table-block.is-wide .stats-table tbody th { white-space: nowrap; }
}
/* Phone: the question mark of an unsure value gets a full 44px target. The
   glyph stays small; only its hit box grows. */
@media (max-width: 820px) {
  :is(.match-card, .game-page) .stat-unsure { --unsure-hit: var(--hit-target-min); }
}
"#;

/// One column of a [`StatsTable`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Col {
    /// Left-aligned text.
    Text(&'static str),
    /// Right-aligned figure.
    Num(&'static str),
    /// Right-aligned win rate with its bar.
    Rate(&'static str),
}

impl Col {
    const fn label(self) -> &'static str {
        match self {
            Col::Text(label) | Col::Num(label) | Col::Rate(label) => label,
        }
    }

    const fn class(self) -> &'static str {
        match self {
            Col::Text(_) => "",
            Col::Num(_) => "num",
            Col::Rate(_) => "rate",
        }
    }
}

/// The stats table. Its scroll container is a region named by `labelled_by`
/// (the id of the visible heading over the table) and takes keyboard focus, so
/// a table wider than the screen scrolls inside it with the arrow keys.
/// `wide` shows the "scroll sideways" hint at phone width.
///
/// Rows are the children: a `th { scope: "row" }` for the name, then `td`
/// cells with the class of their column ([`rate_cell`] for a win rate).
#[component]
pub(crate) fn StatsTable(
    labelled_by: String,
    cols: Vec<Col>,
    #[props(default = false)] wide: bool,
    children: Element,
) -> Element {
    rsx! {
        crate::styles::StyleOnce { id: STATS_VIEWS_CSS_ID, css: STATS_VIEWS_CSS }
        div { class: if wide { "stats-table-block is-wide" } else { "stats-table-block" },
            if wide {
                p { class: "stats-table-hint", "Scroll sideways to see every column." }
            }
            div {
                class: "stats-table-wrap",
                role: "region",
                aria_labelledby: "{labelled_by}",
                tabindex: "0",
                table { class: "stats-table",
                    thead {
                        tr {
                            for col in cols.iter() {
                                th { key: "{col.label()}", scope: "col", class: col.class(), "{col.label()}" }
                            }
                        }
                    }
                    tbody { {children} }
                }
            }
        }
    }
}

/// The win-rate cell of a [`StatsTable`] row: the result as text (a rate from
/// 10 decided games, the record below that), plus a neutral bar under a rate.
/// The bar is hidden from assistive tech because the number says the same.
pub(crate) fn rate_cell(tally: Tally, text: &str, class: &str) -> Element {
    let text = text.to_string();
    let class = class.to_string();
    let rate = tally.win_rate_pct();
    rsx! {
        td { class: "rate",
            div { class: "stats-rate",
                span { class: "{class}", "{text}" }
                if let Some(pct) = rate {
                    span { class: "stats-rate-bar", aria_hidden: "true",
                        span { class: "stats-rate-fill", style: "width:{pct:.1}%;" }
                    }
                }
            }
        }
    }
}

/// Failed load of one stats block: plain words plus Retry.
pub(crate) fn stats_load_error(what: &'static str, mut refresh: Signal<u64>) -> Element {
    rsx! {
        LoadError {
            message: "Couldn't load {what}. Check your connection and try again.",
            on_retry: move |_| refresh += 1,
        }
    }
}

const MEMBER_STATS_CSS: &str = r#"
.member-stats .member-stats-season { max-width: calc(var(--space-12) * 6); min-width: 0; }
.member-stats .stats-summary {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-3);
  min-width: 0;
}
.member-stats .stat-tile {
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-1);
  min-width: 0;
  padding: var(--card-padding);
  background: var(--card-bg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
  text-align: center;
}
.member-stats .stat-tile-value {
  font-family: var(--font-head);
  font-weight: var(--font-head-weight);
  font-size: var(--text-2xl);
  letter-spacing: var(--font-head-tracking);
  line-height: var(--leading-display);
  font-variant-numeric: tabular-nums;
  color: var(--text);
}
.member-stats .stat-tile-label {
  color: var(--text-3);
  font-size: var(--text-sm);
  line-height: var(--leading-ui);
}
.member-stats .stat-tile-record {
  color: var(--text-3);
  font-size: var(--text-xs);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.member-stats .member-stats-panel:focus-visible {
  outline: var(--focus-ring-width) solid var(--focus-ring);
  outline-offset: var(--focus-ring-offset);
}
"#;

/// What the whole page answers before any tab matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemberView {
    /// Private or members only: the copy to show.
    Denied(&'static str),
    /// 404: no such member.
    Missing,
    /// The summary decides (loading, sign in, data).
    Open,
}

fn member_view(summary_error: Option<&str>) -> MemberView {
    match summary_error {
        Some(err) if is_http_status(err, 404) => MemberView::Missing,
        Some(err) => match stats_privacy::member_stats_denied(err) {
            Some(copy) => MemberView::Denied(copy),
            None => MemberView::Open,
        },
        None => MemberView::Open,
    }
}

/// The line screen readers hear for the open tab. Always rendered, only its
/// text changes.
fn panel_status(what: &str, loading: bool, failed: bool, empty: bool) -> String {
    if failed {
        format!("Couldn't load {what}.")
    } else if loading {
        format!("Loading {what}.")
    } else if empty {
        format!("No {what} yet.")
    } else {
        String::new()
    }
}

#[component]
pub fn StatsMember(id: String) -> Element {
    let auth = use_auth();
    let member_id = id.clone();
    let member_id_h = id.clone();
    let member_id_m = id.clone();
    let member_id_r = id.clone();

    // Same saved season as My Stats and leaderboards, including the
    // "current" sentinel. Nothing saved stays all time.
    let season = use_stats_season();

    let stats = use_api_with::<PersonalStats>(move || {
        season.fetch_path(&format!("/api/stats/member/{member_id}"))
    });

    let heroes = use_api_with::<Vec<HeroStats>>(move || {
        season.fetch_path(&format!("/api/stats/member/{member_id_h}/heroes"))
    });

    let maps = use_api_with::<Vec<MapStats>>(move || {
        season.fetch_path(&format!("/api/stats/member/{member_id_m}/maps"))
    });

    let roles = use_api_with::<Vec<scuffed_types::RoleStats>>(move || {
        season.fetch_path(&member_roles_path(&member_id_r))
    });

    let mut tab = use_signal(|| MemberStatsTab::Overview);

    // A refused summary (private, members only, bad id) or a missing member
    // is the whole page's answer. Say so instead of showing tabs that never
    // load.
    let view = member_view(stats.error.read().as_deref());
    match view {
        MemberView::Missing => {
            return rsx! {
                PageShell { width: PageWidth::Wide, class: "member-stats",
                    NotFoundPanel {
                        title: "Member not found",
                        message: "There is no stats page for this member. The link may be wrong, or the member has left.",
                        links: vec![
                            NotFoundLink { label: "See the leaderboards", to: Route::Leaderboards {} },
                            NotFoundLink { label: "Back to your stats", to: Route::Stats {} },
                        ],
                    }
                }
            };
        }
        MemberView::Denied(copy) => {
            return rsx! {
                PageShell { width: PageWidth::Wide, class: "member-stats",
                    Wall {
                        title: "Stats not shown",
                        lead: copy,
                        page_title: true,
                        actions: rsx! {
                            Link { to: Route::Leaderboards {}, class: btn_class(true), "See the leaderboards" }
                            Link { to: Route::Stats {}, class: btn_class(false), "Back to your stats" }
                        },
                    }
                }
            };
        }
        MemberView::Open => {}
    }

    rsx! {
        crate::styles::StyleOnce { id: STATS_VIEWS_CSS_ID, css: STATS_VIEWS_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-stats_member-member-stats-css", css: MEMBER_STATS_CSS }

        PageShell { width: PageWidth::Wide, class: "member-stats",
            PageHeader {
                title: "Member stats",
                lead: "Results from this member's uploaded games. A rate needs 10 decided games.",
            }
            div { class: "member-stats-season",
                SeasonSelect {
                    id: "member-stats-season".to_string(),
                    name: "member-stats-season".to_string(),
                    seasons: season.season_list(),
                    seasons_error: season.seasons_error(),
                    on_retry: move |_| season.retry(),
                    value: season.selected_id(),
                    onchange: move |s| season.choose(s),
                }
            }

            {
                let data = stats.data.read();
                let s = data.as_ref().and_then(|d| d.as_ref());
                match s {
                    None => member_pending(&auth(), &stats, "member stats"),
                    Some(s) if s.total_matches == 0 => rsx! {
                        EmptyCard {
                            title: "No games tracked yet",
                            message: "This member has not uploaded any games for this season. Try another season, or see who has on the leaderboards.",
                            actions: rsx! {
                                Link { to: Route::Leaderboards {}, class: btn_class(true), "See the leaderboards" }
                            },
                        }
                    },
                    Some(s) => {
                        let season = Tally::new(s.total_matches, s.wins, s.losses, s.draws);
                        rsx! {
                            div { class: "stats-summary",
                                {crate::sample::season_tile(season)}
                                div { class: "stat-tile",
                                    div { class: "stat-tile-value", "{s.total_matches}" }
                                    div { class: "stat-tile-label", "Matches" }
                                }
                            }
                        }
                    }
                }
            }

            div { class: "chip-row is-scroll", role: "tablist", aria_label: "Stats sections",
                TabButton {
                    class: "chip".to_string(),
                    label: "Overview".to_string(),
                    id: member_tab_id(MemberStatsTab::Overview).to_string(),
                    controls: MEMBER_STATS_PANEL.to_string(),
                    selected: tab() == MemberStatsTab::Overview,
                    onclick: move |_| tab.set(MemberStatsTab::Overview),
                }
                TabButton {
                    class: "chip".to_string(),
                    label: "Heroes".to_string(),
                    id: member_tab_id(MemberStatsTab::Heroes).to_string(),
                    controls: MEMBER_STATS_PANEL.to_string(),
                    selected: tab() == MemberStatsTab::Heroes,
                    onclick: move |_| tab.set(MemberStatsTab::Heroes),
                }
                TabButton {
                    class: "chip".to_string(),
                    label: "Maps".to_string(),
                    id: member_tab_id(MemberStatsTab::Maps).to_string(),
                    controls: MEMBER_STATS_PANEL.to_string(),
                    selected: tab() == MemberStatsTab::Maps,
                    onclick: move |_| tab.set(MemberStatsTab::Maps),
                }
            }

            div {
                id: MEMBER_STATS_PANEL,
                class: "member-stats-panel ui-stack",
                role: "tabpanel",
                aria_labelledby: member_tab_id(tab()),
                tabindex: "0",
                match tab() {
                    MemberStatsTab::Overview => rsx! {
                        {
                            let err = roles.error.read().clone();
                            let data = roles.data.read();
                            let rows = data.as_ref().and_then(|d| d.as_ref());
                            if err.as_deref().is_some_and(hide_member_role_error) {
                                rsx! { p { class: "visually-hidden", role: "status" } }
                            } else {
                                let panel = role_panel(err.as_deref(), rows.map(|rows| rows.as_slice()));
                                let shown: Vec<_> = match &panel {
                                    RolePanel::Ready(rows) => role_aggs_from_rows(rows)
                                        .into_iter()
                                        .filter(|r| r.matches > 0)
                                        .collect(),
                                    _ => Vec::new(),
                                };
                                let status = panel_status(
                                    "role stats",
                                    matches!(panel, RolePanel::Pending),
                                    matches!(panel, RolePanel::Failed),
                                    shown.is_empty(),
                                );
                                rsx! {
                                    h2 { id: "member-stats-roles-title", class: "ui-section-title", "Roles" }
                                    p { class: "visually-hidden", role: "status", "{status}" }
                                    match panel {
                                        RolePanel::Pending => rsx! { SkeletonBars { rows: 3 } },
                                        RolePanel::Failed => stats_load_error("the role breakdown", roles.refresh),
                                        RolePanel::Ready(_) if shown.is_empty() => rsx! {
                                            p { class: "stats-view-note", "No role stats yet. They show up once this member has tracked games on a role." }
                                        },
                                        RolePanel::Ready(_) => rsx! {
                                            StatsTable {
                                                labelled_by: "member-stats-roles-title",
                                                cols: vec![Col::Text("Role"), Col::Num("Matches"), Col::Rate("Win rate")],
                                                for agg in shown.iter() {
                                                    {
                                                        let tally = agg.tally();
                                                        let result = role_result(tally);
                                                        let name = agg.name;
                                                        let matches = agg.matches;
                                                        rsx! {
                                                            tr { key: "{name}",
                                                                th { scope: "row", "{name}" }
                                                                td { class: "num", "{matches}" }
                                                                {rate_cell(tally, &result, result_class(tally))}
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        },
                                    }
                                }
                            }
                        }
                    },
                    MemberStatsTab::Heroes => rsx! {
                        {
                            let failed = heroes.error.read().is_some();
                            let data = heroes.data.read();
                            let list = data.as_ref().and_then(|d| d.as_ref());
                            let status = panel_status(
                                "hero stats",
                                list.is_none(),
                                list.is_none() && failed,
                                list.is_some_and(|l| l.is_empty()),
                            );
                            rsx! {
                                h2 { id: "member-stats-heroes-title", class: "ui-section-title", "Heroes" }
                                p { class: "visually-hidden", role: "status", "{status}" }
                                match list {
                                    None if failed => stats_load_error("hero stats", heroes.refresh),
                                    None => rsx! { SkeletonBars { rows: 4 } },
                                    Some(list) if list.is_empty() => rsx! {
                                        p { class: "stats-view-note", "No hero stats yet. They show up once this member has tracked games." }
                                    },
                                    Some(list) => {
                                        let all: Vec<&HeroStats> = list.iter().collect();
                                        // Under 3 games a hero folds into one "Other heroes" row.
                                        let (mut sorted, folded) = fold_small(&all, |h| h.tally());
                                        sorted.sort_by_key(|b| std::cmp::Reverse(b.matches));
                                        let other = Tally::sum(folded.iter().map(|h| h.tally()));
                                        let other_count = folded.len();
                                        let other_names = folded
                                            .iter()
                                            .map(|h| h.hero.as_str())
                                            .collect::<Vec<_>>()
                                            .join(", ");
                                        // The pool mixes heroes, so it always shows the record, muted.
                                        let other_result = other_label(other);
                                        rsx! {
                                            p { class: "stats-view-note", {HERO_RULE_NOTE} }
                                            StatsTable {
                                                labelled_by: "member-stats-heroes-title",
                                                wide: true,
                                                cols: vec![
                                                    Col::Text("Hero"),
                                                    Col::Num("Matches"),
                                                    Col::Rate("Win rate"),
                                                    Col::Num("Avg elims"),
                                                    Col::Num("Avg deaths"),
                                                    Col::Num("Avg damage"),
                                                    Col::Num("Avg healing"),
                                                ],
                                                for hero in sorted.iter() {
                                                    {
                                                        let tally = hero.tally();
                                                        let result = tally.label();
                                                        rsx! {
                                                            tr { key: "{hero.hero}",
                                                                th { scope: "row", "{hero.hero}" }
                                                                td { class: "num", "{hero.matches}" }
                                                                {rate_cell(tally, &result, result_class(tally))}
                                                                td { class: "num", "{hero.avg_elims:.1}" }
                                                                td { class: "num", "{hero.avg_deaths:.1}" }
                                                                td { class: "num", "{hero.avg_damage:.0}" }
                                                                td { class: "num", "{hero.avg_healing:.0}" }
                                                            }
                                                        }
                                                    }
                                                }
                                                if other_count > 0 {
                                                    tr { key: "{OTHER_HEROES}", class: "stats-row-muted", title: "{other_names}",
                                                        th { scope: "row", "{OTHER_HEROES} ({other_count})" }
                                                        td { class: "num", "{other.games}" }
                                                        td { class: "rate", span { class: "{OTHER_RESULT_CLASS}", "{other_result}" } }
                                                        td { class: "num", "-" }
                                                        td { class: "num", "-" }
                                                        td { class: "num", "-" }
                                                        td { class: "num", "-" }
                                                    }
                                                }
                                            }
                                        }
                                    },
                                }
                            }
                        }
                    },
                    MemberStatsTab::Maps => rsx! {
                        {
                            let failed = maps.error.read().is_some();
                            let data = maps.data.read();
                            let list = data.as_ref().and_then(|d| d.as_ref());
                            let status = panel_status(
                                "map stats",
                                list.is_none(),
                                list.is_none() && failed,
                                list.is_some_and(|l| l.is_empty()),
                            );
                            rsx! {
                                h2 { id: "member-stats-maps-title", class: "ui-section-title", "Maps" }
                                p { class: "visually-hidden", role: "status", "{status}" }
                                match list {
                                    None if failed => stats_load_error("map stats", maps.refresh),
                                    None => rsx! { SkeletonBars { rows: 4 } },
                                    Some(list) if list.is_empty() => rsx! {
                                        p { class: "stats-view-note", "No map stats yet. They show up once this member has tracked games." }
                                    },
                                    Some(list) => {
                                        let mut sorted: Vec<_> = list.iter().collect();
                                        sorted.sort_by_key(|b| std::cmp::Reverse(b.matches));
                                        rsx! {
                                            p { class: "stats-view-note", {MAP_RULE_NOTE} }
                                            StatsTable {
                                                labelled_by: "member-stats-maps-title",
                                                wide: true,
                                                cols: vec![
                                                    Col::Text("Map"),
                                                    Col::Num("Matches"),
                                                    Col::Rate("Win rate"),
                                                    Col::Num("Wins"),
                                                    Col::Num("Losses"),
                                                    Col::Num("Draws"),
                                                ],
                                                for map in sorted.iter() {
                                                    {
                                                        let tally = map.tally();
                                                        let result = tally.label();
                                                        rsx! {
                                                            tr { key: "{map.map_name}",
                                                                th { scope: "row", "{map.map_name}" }
                                                                td { class: "num", "{map.matches}" }
                                                                {rate_cell(tally, &result, result_class(tally))}
                                                                td { class: "num", "{map.wins}" }
                                                                td { class: "num", "{map.losses}" }
                                                                td { class: "num", "{map.draws}" }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    },
                                }
                            }
                        }
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_tab_ids_are_unique_and_differ_from_the_panel() {
        let tabs = [
            MemberStatsTab::Overview,
            MemberStatsTab::Heroes,
            MemberStatsTab::Maps,
        ];
        let ids: Vec<&str> = tabs.iter().map(|t| member_tab_id(*t)).collect();
        for (i, id) in ids.iter().enumerate() {
            assert!(!ids[i + 1..].contains(id), "{id}");
            assert_ne!(*id, MEMBER_STATS_PANEL);
        }
    }

    #[test]
    fn a_missing_member_is_not_found_and_a_refusal_is_a_wall() {
        assert_eq!(
            member_view(Some("HTTP error 404: Member not found")),
            MemberView::Missing
        );
        assert_eq!(
            member_view(Some("HTTP error 403: stats_private")),
            MemberView::Denied("This member keeps their stats private.")
        );
        assert!(matches!(
            member_view(Some("HTTP error 400: bad id")),
            MemberView::Denied(_)
        ));
        // Sign in, inactive and server errors stay with the member gate.
        for err in [
            "HTTP error 401: Unauthorized",
            "HTTP error 500: Internal error",
            "Network error: offline",
        ] {
            assert_eq!(member_view(Some(err)), MemberView::Open, "{err}");
        }
        assert_eq!(member_view(None), MemberView::Open);
    }

    #[test]
    fn panel_status_names_the_state_and_is_empty_when_rows_show() {
        assert_eq!(
            panel_status("hero stats", true, false, false),
            "Loading hero stats."
        );
        assert_eq!(
            panel_status("hero stats", true, true, false),
            "Couldn't load hero stats."
        );
        assert_eq!(
            panel_status("hero stats", false, false, true),
            "No hero stats yet."
        );
        assert_eq!(panel_status("hero stats", false, false, false), "");
    }

    fn render(app: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn table_scrolls_inside_a_named_focusable_region() {
        fn view() -> Element {
            let ten = Tally::new(10, 7, 3, 0);
            let two = Tally::new(2, 1, 1, 0);
            rsx! {
                h2 { id: "t-title", "Heroes" }
                StatsTable {
                    labelled_by: "t-title",
                    wide: true,
                    cols: vec![Col::Text("Hero"), Col::Num("Matches"), Col::Rate("Win rate")],
                    tr {
                        th { scope: "row", "Ana" }
                        td { class: "num", "10" }
                        {rate_cell(ten, &ten.label(), result_class(ten))}
                    }
                    tr {
                        th { scope: "row", "Kiriko" }
                        td { class: "num", "2" }
                        {rate_cell(two, &two.label(), result_class(two))}
                    }
                }
            }
        }
        let html = render(view);
        assert!(
            html.contains(
                "<div class=\"stats-table-wrap\" role=\"region\" aria-labelledby=\"t-title\" tabindex=\"0\">"
            ),
            "{html}"
        );
        assert!(
            html.contains("Scroll sideways to see every column."),
            "{html}"
        );
        assert_eq!(html.matches("scope=\"col\"").count(), 3, "{html}");
        assert_eq!(html.matches("<th scope=\"row\">").count(), 2, "{html}");
        // A rate is a number first. Its bar is decoration for assistive tech.
        assert!(
            html.contains("<span class=\"stats-wr\">70.0%</span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"stats-rate-bar\" aria-hidden=\"true\"><span class=\"stats-rate-fill\" style=\"width:70.0%;\">"),
            "{html}"
        );
        // Below the gate the record shows, muted, with no bar.
        assert!(
            html.contains("<span class=\"stats-wr muted\">1W-1L</span>"),
            "{html}"
        );
        assert_eq!(html.matches("stats-rate-bar").count(), 1, "{html}");
    }

    #[test]
    fn shared_css_uses_tokens_only_and_never_dims_text_with_opacity() {
        for css in [STATS_VIEWS_CSS, MEMBER_STATS_CSS] {
            for line in css.lines() {
                let decl = line.trim();
                if decl.starts_with("/*") || decl.starts_with("@media") || !decl.contains(':') {
                    continue;
                }
                assert!(
                    !decl.contains("rem") && !decl.contains("#"),
                    "size or colour literal: {decl}"
                );
                let px = decl.contains("px")
                    && !decl.starts_with("border")
                    && !decl.contains("1px solid");
                assert!(!px, "pixel literal: {decl}");
            }
            assert!(!css.contains('\u{2014}') && !css.contains('\u{2013}'));
        }
        assert!(STATS_VIEWS_CSS.contains(".stats-table tr.stats-row-muted { opacity: 1; }"));
        assert!(STATS_VIEWS_CSS.contains(".stats-table-wrap:focus-visible {"));
    }
}
