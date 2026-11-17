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

/* The win-rate bar charts of Heroes and Maps, drawn as the role bars of the
   stats frame: the name, a slim pill track, then the figure. The shared chart
   (components/charts.rs) sets its text at 0.8rem, which is 11.2px on the 14px
   root and under the 12px floor; inside these cards the text is on the type
   scale instead. A small sample stays quieter through the chart's own muted
   ink, never through a smaller size. */
.stats-view-card .hbar-list { gap: var(--space-3); }
.stats-view-card .hbar-row {
  display: grid;
  grid-template-columns: calc(var(--space-8) * 4) minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--space-1) var(--space-3);
  min-width: 0;
}
.stats-view-card .hbar-label {
  min-width: 0;
  text-align: left;
  font-size: var(--text-md);
  color: var(--text);
}
.stats-view-card .hbar-track {
  height: var(--space-3);
  background: var(--surface-sunken);
  border-radius: var(--radius-pill);
}
.stats-view-card .hbar-fill { border-radius: var(--radius-pill); }
.stats-view-card .hbar-val {
  min-width: 0;
  text-align: right;
  font-size: var(--text-sm);
  color: var(--text);
}
@media (max-width: 720px) {
  .stats-view-card .hbar-row { grid-template-columns: minmax(0, 1fr) auto; }
  .stats-view-card .hbar-label { grid-column: 1 / -1; }
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
    /// Right-aligned row buttons, so the header sits over them.
    Actions(&'static str),
}

impl Col {
    const fn label(self) -> &'static str {
        match self {
            Col::Text(label) | Col::Num(label) | Col::Rate(label) | Col::Actions(label) => label,
        }
    }

    const fn class(self) -> &'static str {
        match self {
            Col::Text(_) => "",
            Col::Num(_) => "num",
            Col::Rate(_) => "rate",
            Col::Actions(_) => "stats-table-actions",
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

/// The words for a failed load of one stats block, and whether Retry can
/// help. A refused request is not a connection problem: a retry gives the
/// same answer, so it gets no button.
fn load_error_copy(what: &str, error: Option<&str>) -> (String, bool) {
    match error {
        Some(err) if is_http_status(err, 401) => (
            format!("Couldn't load {what}: you are logged out. Log in again to see it."),
            false,
        ),
        Some(err) if is_http_status(err, 403) => (
            format!("Couldn't load {what}: your account can't see it."),
            false,
        ),
        _ => (
            format!("Couldn't load {what}. Check your connection and try again."),
            true,
        ),
    }
}

/// The panel's live status line, or the refusal when the load was refused.
/// A refused load (401, 403) has no Retry, and a card that appears with its
/// text already in it is not reliably announced, so the status line that
/// was on the page since loading says it, once. Other failures keep `status`
/// (empty): their card is an alert. `error` is `None` unless the block
/// failed with nothing to show.
pub(crate) fn status_or_refusal(status: String, what: &str, error: Option<&str>) -> String {
    match (error, load_error_copy(what, error)) {
        (Some(_), (text, false)) => text,
        _ => status,
    }
}

/// Failed load of one stats block: plain words, plus Retry when a retry can
/// help. `error` is the fetch error text. A refused load renders the same
/// card without a live role: [`status_or_refusal`] announces it.
pub(crate) fn stats_load_error(
    what: &'static str,
    error: Option<String>,
    mut refresh: Signal<u64>,
) -> Element {
    let (message, can_retry) = load_error_copy(what, error.as_deref());
    if !can_retry {
        return rsx! {
            div { class: "fetch-error-wrap",
                p { class: "fetch-error", "{message}" }
            }
        };
    }
    rsx! {
        LoadError {
            message,
            on_retry: EventHandler::new(move |_: ()| refresh += 1),
        }
    }
}

const MEMBER_STATS_CSS: &str = r#"
/* One row under the page header, as on Your stats: the tabs, then the season. */
.member-stats .member-stats-head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3) var(--space-4);
  min-width: 0;
}
.member-stats .member-stats-tabs { flex: 0 1 auto; }
.member-stats .member-stats-season { flex: 0 1 auto; min-width: 0; max-width: 100%; }
/* The season select is a 44px control, as on Your stats and leaderboards. */
.member-stats .member-stats-season select { min-height: var(--hit-target-min); }
/* The tiles sit in the panel stack. A section gap, not a stack step, parts
   them from the section title under them. */
.member-stats .member-stats-summary {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
  margin-bottom: calc(var(--space-section) - var(--space-stack));
}
.member-stats .member-stats-panel:focus-visible {
  outline: var(--focus-ring-width) solid var(--focus-ring);
  outline-offset: var(--focus-ring-offset);
}
@media (max-width: 720px) {
  .member-stats .member-stats-season,
  .member-stats .member-stats-season .season-select,
  .member-stats .member-stats-season select {
    flex: 1 1 100%;
    width: 100%;
  }
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

/// The zero-game card's message. All time has no season to name.
fn member_no_games_message(season: Option<&str>) -> &'static str {
    match season {
        None => "This member has not uploaded any games yet. See who has on the leaderboards.",
        Some(_) => {
            "This member has not uploaded any games for this season. Try another season, or see who has on the leaderboards."
        }
    }
}

/// The line screen readers hear for the open tab. Always rendered, only its
/// text changes.
/// A failure says nothing here: its card is an alert that already says it.
fn panel_status(what: &str, loading: bool, failed: bool, empty: bool) -> String {
    if failed {
        String::new()
    } else if loading {
        format!("Loading {what}.")
    } else if empty {
        format!("No {what} yet.")
    } else {
        String::new()
    }
}

/// The summary tiles of the frame: win rate and matches, value over label.
/// The win rate follows the small-sample rule in [`crate::sample`]: under 10
/// decided games the tile shows the record and reads "Too few games". From 10
/// on, the record sits in a line under the row, as on Your stats.
fn member_summary(season: Tally) -> Element {
    let rate = season.label();
    let (rate_label, record) = if season.has_win_rate() {
        ("Win rate", Some(season.record()))
    } else {
        (crate::sample::TOO_FEW_GAMES, None)
    };
    rsx! {
        div { class: "member-stats-summary",
            div { class: "stats-view-tiles",
                div { class: "stats-view-tile",
                    div { class: "stats-view-tile-value", "{rate}" }
                    div { class: "stats-view-tile-label", "{rate_label}" }
                }
                div { class: "stats-view-tile",
                    div { class: "stats-view-tile-value", "{season.games}" }
                    div { class: "stats-view-tile-label", "Matches" }
                }
            }
            if let Some(record) = record {
                p { class: "stats-view-note", "Record {record}." }
            }
        }
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
            div { class: "member-stats-head",
                div { class: "chip-row is-scroll member-stats-tabs", role: "tablist", aria_label: "Stats sections",
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
            }

            div {
                id: MEMBER_STATS_PANEL,
                class: "member-stats-panel ui-stack",
                role: "tabpanel",
                aria_labelledby: member_tab_id(tab()),
                tabindex: "0",
                {
                    let data = stats.data.read();
                    let s = data.as_ref().and_then(|d| d.as_ref());
                    match s {
                        None => member_pending(&auth(), &stats, "member stats"),
                        Some(s) if s.total_matches == 0 => rsx! {
                            EmptyCard {
                                title: "No games tracked yet",
                                message: member_no_games_message(season.selected_id().as_deref()),
                                actions: rsx! {
                                    Link { to: Route::Leaderboards {}, class: btn_class(true), "See the leaderboards" }
                                },
                            }
                        },
                        Some(s) => {
                            let season = Tally::new(s.total_matches, s.wins, s.losses, s.draws);
                            member_summary(season)
                        }
                    }
                }

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
                                let status = status_or_refusal(
                                    panel_status(
                                        "role stats",
                                        matches!(panel, RolePanel::Pending),
                                        matches!(panel, RolePanel::Failed),
                                        shown.is_empty(),
                                    ),
                                    "the role breakdown",
                                    err.as_deref().filter(|_| matches!(panel, RolePanel::Failed)),
                                );
                                rsx! {
                                    h2 { id: "member-stats-roles-title", class: "ui-section-title", "Roles" }
                                    p { class: "visually-hidden", role: "status", "{status}" }
                                    match panel {
                                        RolePanel::Pending => rsx! { SkeletonBars { rows: 3 } },
                                        RolePanel::Failed => stats_load_error("the role breakdown", err.clone(), roles.refresh),
                                        RolePanel::Ready(_) if shown.is_empty() => rsx! {
                                            p { class: "stats-view-note", "No role stats yet. They show up once this member has tracked games on a role." }
                                        },
                                        RolePanel::Ready(_) => rsx! {
                                            StatsTable {
                                                labelled_by: "member-stats-roles-title",
                                                cols: vec![Col::Text("Role"), Col::Num("Matches"), Col::Rate("Win rate")],
                                                for agg in shown.iter() {
                                                    Fragment { key: "{agg.name}", {
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
                                                    } }
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
                            let hero_err = heroes.error.read().clone();
                            let status = status_or_refusal(
                                panel_status(
                                    "hero stats",
                                    list.is_none(),
                                    list.is_none() && failed,
                                    list.is_some_and(|l| l.is_empty()),
                                ),
                                "hero stats",
                                hero_err.as_deref().filter(|_| list.is_none()),
                            );
                            rsx! {
                                h2 { id: "member-stats-heroes-title", class: "ui-section-title", "Heroes" }
                                p { class: "visually-hidden", role: "status", "{status}" }
                                match list {
                                    None if failed => stats_load_error("hero stats", heroes.error.read().clone(), heroes.refresh),
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
                                                    Fragment { key: "{hero.hero}", {
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
                                                    } }
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
                            let map_err = maps.error.read().clone();
                            let status = status_or_refusal(
                                panel_status(
                                    "map stats",
                                    list.is_none(),
                                    list.is_none() && failed,
                                    list.is_some_and(|l| l.is_empty()),
                                ),
                                "map stats",
                                map_err.as_deref().filter(|_| list.is_none()),
                            );
                            rsx! {
                                h2 { id: "member-stats-maps-title", class: "ui-section-title", "Maps" }
                                p { class: "visually-hidden", role: "status", "{status}" }
                                match list {
                                    None if failed => stats_load_error("map stats", maps.error.read().clone(), maps.refresh),
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
                                                    Fragment { key: "{map.map_name}", {
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
                                                    } }
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
    fn all_time_names_no_season() {
        assert!(!member_no_games_message(None).contains("season"));
        assert!(member_no_games_message(Some("s1")).contains("this season"));
    }

    #[test]
    fn a_refused_load_is_announced_once_by_the_status_line() {
        let forbidden = Some("HTTP error 403: Forbidden");
        let said = status_or_refusal(String::new(), "hero stats", forbidden);
        assert!(said.contains("your account can't see it"), "{said}");
        // A retryable failure keeps the (empty) line: its card is the alert.
        assert_eq!(
            status_or_refusal(String::new(), "hero stats", Some("HTTP error 500: x")),
            ""
        );
        assert_eq!(
            status_or_refusal("Loading hero stats.".into(), "hero stats", None),
            "Loading hero stats."
        );
        // The refused card itself has no live role, so it is not read twice;
        // a retryable card stays an alert with Retry.
        fn refused() -> Element {
            stats_load_error(
                "hero stats",
                Some("HTTP error 401: Unauthorized".into()),
                use_signal(|| 0),
            )
        }
        fn broken() -> Element {
            stats_load_error(
                "hero stats",
                Some("HTTP error 500: x".into()),
                use_signal(|| 0),
            )
        }
        fn card(app: fn() -> Element) -> String {
            let mut dom = VirtualDom::new(app);
            dom.rebuild_in_place();
            dioxus_ssr::render(&dom)
        }
        let html = card(refused);
        assert!(html.contains("Log in again"), "{html}");
        assert!(!html.contains("role="), "{html}");
        assert!(!html.contains("<button"), "{html}");
        let html = card(broken);
        assert!(html.contains(r#"role="alert""#), "{html}");
        assert!(html.contains("<button"), "{html}");
        // Every panel that can show the card routes its status through it.
        for (file, src) in [
            ("stats_member.rs", include_str!("stats_member.rs")),
            ("heroes.rs", include_str!("stats/heroes.rs")),
            ("maps.rs", include_str!("stats/maps.rs")),
            ("history.rs", include_str!("stats/history.rs")),
        ] {
            let prod = &src[..src.find("#[cfg(test)]").unwrap()];
            assert_eq!(
                prod.matches("stats_load_error(").count() - usize::from(file == "stats_member.rs"),
                prod.matches("status_or_refusal(").count() - usize::from(file == "stats_member.rs"),
                "{file}"
            );
        }
    }

    #[test]
    fn a_refused_load_is_not_a_connection_problem() {
        let (text, retry) = load_error_copy("hero stats", Some("HTTP error 403: Forbidden"));
        assert!(!text.contains("connection"), "{text}");
        assert!(!retry, "a retry gets the same 403");
        let (text, retry) = load_error_copy("map stats", Some("HTTP error 401: Unauthorized"));
        assert!(
            !text.contains("connection") && text.contains("Log in again"),
            "{text}"
        );
        assert!(!retry);
        let (text, retry) = load_error_copy("map stats", Some("HTTP error 500: Internal error"));
        assert!(text.contains("Check your connection"), "{text}");
        assert!(retry);
        let (_, retry) = load_error_copy("map stats", None);
        assert!(retry);
    }

    #[test]
    fn panel_status_names_the_state_and_is_empty_when_rows_show() {
        assert_eq!(
            panel_status("hero stats", true, false, false),
            "Loading hero stats."
        );
        // The failure card is the alert: the status line does not repeat it.
        assert_eq!(panel_status("hero stats", true, true, false), "");
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

    /// Text of the first rule that starts with `selector`, up to its `}`.
    fn rule<'a>(css: &'a str, selector: &str) -> &'a str {
        let start = css
            .find(selector)
            .unwrap_or_else(|| panic!("missing rule {selector}"));
        let rest = &css[start..];
        &rest[..rest.find('}').unwrap()]
    }

    #[test]
    fn bar_chart_text_is_on_the_type_scale_inside_stats_cards() {
        // The shared chart sets 0.8rem (11.2px, under the 12px floor). Inside
        // a stats card the label and figure take type tokens, muted rows too.
        let label = rule(STATS_VIEWS_CSS, ".stats-view-card .hbar-label {");
        assert!(label.contains("font-size: var(--text-md)"), "{label}");
        let val = rule(STATS_VIEWS_CSS, ".stats-view-card .hbar-val {");
        assert!(val.contains("font-size: var(--text-sm)"), "{val}");
        assert!(
            !STATS_VIEWS_CSS.contains(".hbar-row.muted .hbar-label {")
                && !STATS_VIEWS_CSS.contains(".hbar-row.muted .hbar-val {"),
            "muted rows keep the size and change only their ink"
        );
        // The track is the slim pill of the frame's role bars.
        let track = rule(STATS_VIEWS_CSS, ".stats-view-card .hbar-track {");
        assert!(track.contains("height: var(--space-3)"), "{track}");
        assert!(
            track.contains("border-radius: var(--radius-pill)"),
            "{track}"
        );
    }

    #[test]
    fn member_summary_is_two_tiles_with_the_record_under_them() {
        fn rated() -> Element {
            member_summary(Tally::new(12, 10, 2, 0))
        }
        fn small() -> Element {
            member_summary(Tally::new(4, 3, 1, 0))
        }
        let html = render(rated);
        assert_eq!(
            html.matches("class=\"stats-view-tile\"").count(),
            2,
            "{html}"
        );
        assert!(html.contains(">83.3%</div>"), "{html}");
        // Sentence case, as on Your stats.
        assert!(
            html.contains(">Win rate</div>") && !html.contains("Win Rate"),
            "{html}"
        );
        assert!(html.contains(">Matches</div>"), "{html}");
        assert!(
            html.contains("<p class=\"stats-view-note\">Record 10W-2L.</p>"),
            "{html}"
        );
        let html = render(small);
        assert!(html.contains("Too few games"), "{html}");
        assert!(!html.contains("Record "), "{html}");
    }

    #[test]
    fn member_season_select_is_a_44px_control_and_fills_a_phone_row() {
        let select = rule(
            MEMBER_STATS_CSS,
            ".member-stats .member-stats-season select {",
        );
        assert!(
            select.contains("min-height: var(--hit-target-min)"),
            "{select}"
        );
        let phone =
            &MEMBER_STATS_CSS[MEMBER_STATS_CSS.find("@media (max-width: 720px)").unwrap()..];
        assert!(
            phone.contains(".member-stats .member-stats-season select"),
            "{phone}"
        );
        assert!(phone.contains("width: 100%"), "{phone}");
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
