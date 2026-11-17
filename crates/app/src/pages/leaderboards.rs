//! Public leaderboards page.
//!
//! One wide board (Win rate, K/D or Games) with a hero and a season filter.
//! Ranked rows are paged eight at a time with the page in the address. On the
//! Win rate board, players under 10 decided games sit in a second list below.

use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;

use crate::components::ui::inline_confirm::focus_element_by_id_soon;
use crate::components::ui::{
    EmptyCard, HeroSelect, LoadError, PageHeader, PageShell, PageWidth, Pager, SeasonSelect,
    SkeletonBars, TabButton, btn_class, clamp_page, page_count, page_slice, show_list_top,
    use_stats_season, use_url_page,
};
use crate::routes::Route;
use crate::sample::{
    LIST_RULE_NOTE, ListResult, RATE_MIN_DECIDED, TOO_FEW_GAMES, list_decided, list_result,
};
use crate::util::encode_query;

use super::stats_member::{STATS_VIEWS_CSS, STATS_VIEWS_CSS_ID};

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct LeaderboardRow {
    member_id: String,
    display_name: String,
    games: u32,
    /// Victories in `games`. Older servers omit the three counts.
    #[serde(default)]
    wins: u32,
    /// Defeats in `games`.
    #[serde(default)]
    losses: u32,
    /// Draws in `games`.
    #[serde(default)]
    draws: u32,
    winrate: f32,
    kd: f64,
}

/// Page styles. The table follows the frame's `table.data` on the `--table-*`
/// tokens, so the row height tracks `data-density`. At phone width it stays a
/// five column table that fits 390; the wrapper scrolls sideways if a row
/// ever does not fit, so the page itself never does.
const PAGE_CSS: &str = r#"
.lb-page .lb-updated {
  margin: var(--space-2) 0 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-body);
}
.lb-page .lb-updated:empty { display: none; }

.lb-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  min-width: 0;
}
.lb-filters {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
}
/* The shared pickers stack a label over the control. Here the label sits
   beside it, as in the frame. */
.lb-filters .hero-select,
.lb-filters .season-select {
  flex-direction: row;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-1);
  min-width: 0;
}
.lb-filters .ui-label {
  color: var(--text);
  font-family: var(--font-body);
  font-size: var(--text-md);
  font-weight: var(--font-body-weight);
  letter-spacing: normal;
  text-transform: none;
}
.lb-filters select.ui-field {
  box-sizing: border-box;
  width: auto;
  max-width: 100%;
  min-width: 0;
  height: var(--hit-target-min);
  min-height: var(--hit-target-min);
  padding: 0 var(--space-3);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-control);
  background: var(--surface);
  color: var(--text);
  font-family: var(--font-body);
  font-size: var(--text-md);
  font-weight: var(--font-body-weight);
}

.lb-status,
.lb-note {
  margin: 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-body);
}
/* Nothing to say: out of the layout, still in the tree so the next text is
   announced. */
.lb-status:empty {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}

/* The table itself is the shared stats table (`STATS_VIEWS_CSS`); only the
   columns this page adds are styled here. */
.lb-page .stats-table .lb-rank { font-variant-numeric: tabular-nums; }
/* The sorted column carries body ink, header and cells, as the frame's
   Win rate column does. The other numbers stay muted. */
.lb-page .stats-table thead th[aria-sort],
.lb-page .stats-table tbody td.num.is-sorted { color: var(--table-row-fg); }
.lb-record.is-muted { color: var(--table-row-muted); }
.lb-page .stats-table .lb-player {
  color: var(--text);
  font-weight: var(--font-label-weight);
  overflow-wrap: anywhere;
}
.lb-page .stats-table .lb-player a {
  display: inline-flex;
  align-items: center;
  min-width: var(--hit-target-min);
  min-height: var(--hit-target-min);
  color: inherit;
  text-decoration: none;
}
.lb-page .stats-table .lb-player a:hover { text-decoration: underline; }

@media (max-width: 820px) {
  .lb-toolbar {
    flex-direction: column;
    align-items: stretch;
  }
  .lb-page .stats-table .rate { width: calc(var(--space-8) * 3); }
  /* Hero and Season share one row, as in the phone frame. Each column is
     as wide as its picker needs and both shrink to fit; the select gives
     way, never the label. */
  .lb-filters {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, max-content));
    justify-content: start;
  }
  .lb-filters .hero-select:has(> select),
  .lb-filters .season-select:has(> select) { flex-wrap: nowrap; }
  .lb-filters .ui-label { flex: none; }
  .lb-filters select.ui-field { flex: 0 1 auto; }
}
"#;

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct LeaderboardPayload {
    rows: Vec<LeaderboardRow>,
    /// Missing on an older response. Empty or unparseable hides the label.
    #[serde(default)]
    cached_at: Option<String>,
}

/// "Updated just now" / "Updated 12s ago" / "Updated 3m ago" / "Updated 2h ago".
///
/// A timestamp ahead of `now` (clock skew) reads as just now. Parsing stays
/// in [`parse_cached_at`]; a missing stamp is not passed in.
fn updated_label(
    cached_at: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let secs = now.signed_duration_since(cached_at).num_seconds().max(0);
    if secs < 5 {
        "Updated just now".into()
    } else if secs < 60 {
        format!("Updated {secs}s ago")
    } else if secs < 3600 {
        format!("Updated {}m ago", secs / 60)
    } else {
        format!("Updated {}h ago", secs / 3600)
    }
}

fn parse_cached_at(raw: Option<&str>) -> Option<chrono::DateTime<chrono::Utc>> {
    let raw = raw.map(str::trim).filter(|s| !s.is_empty())?;
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// `Hold` is the skipped request while a saved season is unresolved. It
/// renders as Loading. It is not a finished failure.
#[derive(Debug, Clone, PartialEq)]
enum LeaderboardLoad {
    Hold,
    Failed,
    Rows(Vec<LeaderboardRow>),
}

/// `Some(Hold)` when the season filter has not resolved yet. A non-empty
/// path is fetched by the caller.
fn leaderboard_hold(path: &str) -> Option<LeaderboardLoad> {
    if path.is_empty() {
        Some(LeaderboardLoad::Hold)
    } else {
        None
    }
}

/// Status line while the board loads.
const LOADING: &str = "Loading leaderboard";
/// Error text of a failed load. Retry sits under it.
const LOAD_FAILED: &str = "Couldn't load the leaderboard. Check your connection and try again.";
/// Empty state title with no hero filter.
const NO_PLAYERS: &str = "No ranked players yet";
/// Empty state title with a hero picked.
const NO_PLAYERS_ON_HERO: &str = "No ranked players on this hero yet";

/// Words for a board with no table: the loading line, the error text or the
/// empty state title. `None` means render the table.
fn leaderboard_message(
    load: Option<&LeaderboardLoad>,
    hero_filtered: bool,
) -> Option<&'static str> {
    match load {
        None | Some(LeaderboardLoad::Hold) => Some(LOADING),
        Some(LeaderboardLoad::Failed) => Some(LOAD_FAILED),
        Some(LeaderboardLoad::Rows(list)) if list.is_empty() && hero_filtered => {
            Some(NO_PLAYERS_ON_HERO)
        }
        Some(LeaderboardLoad::Rows(list)) if list.is_empty() => Some(NO_PLAYERS),
        Some(LeaderboardLoad::Rows(_)) => None,
    }
}

/// WR cell under the small-sample rule: the win rate from 10 decided games, the
/// record below that, muted under 3 games. Display only: the row order
/// is whatever the server sent.
fn wr_cell(row: &LeaderboardRow) -> ListResult {
    list_result(row.games, row.wins, row.losses, row.draws, row.winrate)
}

/// Width of the bar under a win rate, as the whole percent the cell shows.
/// `None` for a record: a record has no bar.
fn rate_bar_pct(wr: &ListResult) -> Option<u8> {
    if !wr.is_rate {
        return None;
    }
    wr.text
        .strip_suffix('%')
        .and_then(|pct| pct.parse::<u8>().ok())
        .map(|pct| pct.min(100))
}

/// Fewest decided games (win, loss or draw) to be ranked on the Win rate
/// board, the same floor as [`wr_cell`]. The server still sends rows from
/// `LEADERBOARD_MIN_GAMES` (5 games); rows below this floor move to the
/// unranked list. The K/D and Games boards are not split.
const WINRATE_RANK_MIN_DECIDED: u32 = RATE_MIN_DECIDED;

/// Lead under the page title.
const PAGE_LEAD: &str =
    "Ranked from uploaded personal stats. A rate needs 10 decided games. Sparse data is normal.";
/// Caption under the ranked Win rate table.
const WINRATE_BOARD_NOTE: &str = "Ranked by win rate from 10 decided games.";
/// Empty state title when the Win rate board has rows but none with 10 decided games.
const WINRATE_NONE_RANKED: &str = "Nobody has 10 decided games yet";
/// Heading of the unranked list under the Win rate board.
const UNRANKED_TITLE: &str = TOO_FEW_GAMES;
/// Caption under the unranked heading.
const UNRANKED_NOTE: &str = "Record so far. Players join the win rate ranking at 10 decided games.";

/// Ranked rows on one page, as in the frame (1 to 8 of 40).
const BOARD_PAGE_SIZE: usize = 8;

/// Rows for one board: `ranked` in the server's order, and `unranked`
/// for the list under the Win rate board.
#[derive(Debug, Clone, PartialEq)]
struct BoardSplit {
    ranked: Vec<LeaderboardRow>,
    unranked: Vec<LeaderboardRow>,
}

/// Split a board's rows. On the Win rate board, rows with fewer than
/// [`WINRATE_RANK_MIN_DECIDED`] decided games leave the ranking and go to
/// `unranked`, most games first, then by name. Ranked rows keep the
/// server's order (by win rate). Other boards keep every row ranked.
fn board_split(metric: &str, rows: &[LeaderboardRow]) -> BoardSplit {
    if metric != "winrate" {
        return BoardSplit {
            ranked: rows.to_vec(),
            unranked: Vec::new(),
        };
    }
    let (ranked, mut unranked): (Vec<_>, Vec<_>) = rows.iter().cloned().partition(|r| {
        list_decided(r.games, r.wins, r.losses, r.draws) >= WINRATE_RANK_MIN_DECIDED
    });
    unranked.sort_by(|a, b| {
        b.games
            .cmp(&a.games)
            .then_with(|| {
                a.display_name
                    .to_lowercase()
                    .cmp(&b.display_name.to_lowercase())
            })
            .then_with(|| a.member_id.cmp(&b.member_id))
    });
    BoardSplit { ranked, unranked }
}

/// Caption under a board's ranked table.
fn board_note(metric: &str) -> &'static str {
    if metric == "winrate" {
        WINRATE_BOARD_NOTE
    } else {
        LIST_RULE_NOTE
    }
}

/// The boards, in tab order: the API's `metric` value and the tab label.
const BOARDS: [(&str, &str); 3] = [("winrate", "Win rate"), ("kd", "K/D"), ("games", "Games")];

/// Tab label of a board. An unknown metric reads as the first board.
fn board_label(metric: &str) -> &'static str {
    BOARDS
        .iter()
        .find(|(value, _)| *value == metric)
        .map_or(BOARDS[0].1, |(_, label)| label)
}

/// The one tabpanel under the board tabs. It also takes focus after a page
/// change.
const LB_PANEL: &str = "leaderboard-panel";
/// Heading of the unranked list. Its table region is named by it.
const UNRANKED_TITLE_ID: &str = "leaderboard-unranked-title";

/// Most rows one board fetch asks for. A full fetch may leave players out.
const BOARD_LIMIT: usize = 50;

/// The pager noun for the ranked table. At the fetch cap the count is how
/// many are shown, not how many players there are.
fn ranked_noun(rows_fetched: usize) -> (&'static str, &'static str) {
    if rows_fetched >= BOARD_LIMIT {
        ("ranked players shown", "ranked player shown")
    } else {
        ("ranked players", "ranked player")
    }
}

/// Id of a board tab button. The tabpanel is labelled by the selected one.
fn metric_tab_id(metric: &str) -> String {
    format!("leaderboard-tab-{metric}")
}

/// Id of the hero picker. The all-heroes button removes itself, so focus
/// moves here, to the control it just reset. The picker passes the same id.
const HERO_PICK_ID: &str = "leaderboard-hero";

/// What the panel shows for one fetch state.
#[derive(Debug, Clone, PartialEq)]
enum BoardView {
    Loading,
    Failed,
    /// The server sent no rows.
    Empty,
    Rows(BoardSplit),
}

/// The panel state for a resource slot holding the board and its load.
fn board_view(slot: Option<&(String, LeaderboardLoad)>) -> BoardView {
    match slot {
        None | Some((_, LeaderboardLoad::Hold)) => BoardView::Loading,
        Some((_, LeaderboardLoad::Failed)) => BoardView::Failed,
        Some((_, LeaderboardLoad::Rows(list))) if list.is_empty() => BoardView::Empty,
        Some((board, LeaderboardLoad::Rows(list))) => BoardView::Rows(board_split(board, list)),
    }
}

#[component]
pub fn Leaderboards() -> Element {
    let mut metric = use_signal(|| "winrate".to_string());
    let mut hero = use_signal(|| None::<String>);
    // Same saved season as My Stats and member stats, including the
    // "current" sentinel. A pick here changes those pages too.
    let season = use_stats_season();
    // The ranked page lives in the address (`?page=2`), as on Blog and News.
    let page = use_url_page();
    // The short list under the Win rate board pages in memory.
    let mut unranked_page = use_signal(|| 1usize);
    // Last stamp that arrived. A filter change and an unresolved season
    // hold do not clear this, so the header keeps its label until the next
    // payload lands. A failed fetch does clear it.
    let mut shown_at = use_signal(|| None::<chrono::DateTime<chrono::Utc>>);
    let mut label_tick = use_signal(|| 0u32);
    // `use_future` spawns on this component's scope. Dioxus drops that task
    // when the page unmounts, which stops the timer.
    let _label_timer = use_future(move || async move {
        loop {
            #[cfg(feature = "web")]
            {
                gloo_timers::future::TimeoutFuture::new(15_000).await;
            }
            #[cfg(not(feature = "web"))]
            {
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            }
            label_tick += 1;
        }
    });
    // Whether the rows on screen were fetched for one hero. It changes when
    // they land, with the rows, so a refetch never pairs the old rows with
    // the new filter (which flashed the wrong empty card).
    let mut shown_for_hero = use_signal(|| false);
    let mut rows = use_resource(move || {
        let m = metric();
        let h = hero();
        let for_hero = h.is_some();
        let mut url = format!("/api/public/leaderboards?metric={m}&limit={BOARD_LIMIT}");
        if let Some(h) = h {
            url.push_str(&format!("&hero={}", encode_query(&h)));
        }
        // Empty while a saved season is unresolved. Do not send it raw.
        let url = season.fetch_path(&url);
        async move {
            let load = async {
                if let Some(held) = leaderboard_hold(&url) {
                    return held;
                }
                match ApiClient::web().fetch::<LeaderboardPayload>(&url).await {
                    Ok(payload) => {
                        shown_at.set(parse_cached_at(payload.cached_at.as_deref()));
                        LeaderboardLoad::Rows(payload.rows)
                    }
                    Err(_) => {
                        shown_at.set(None);
                        LeaderboardLoad::Failed
                    }
                }
            }
            .await;
            shown_for_hero.set(for_hero);
            // The board travels with its rows, so a tab switch never splits
            // the previous board's rows by the new board's rule.
            (m, load)
        }
    });
    let hero_filtered = shown_for_hero();
    let (message, view, board) = {
        let snapshot = rows.read();
        let message = leaderboard_message(snapshot.as_ref().map(|(_, load)| load), hero_filtered);
        let board = match snapshot.as_ref() {
            Some((board, _)) => board.clone(),
            None => metric(),
        };
        (message, board_view(snapshot.as_ref()), board)
    };
    // A fetch is running: the first load, a retry, or a filter change that
    // still shows the previous rows.
    let busy = rows.state()() == UseResourceState::Pending;
    let _tick = label_tick();
    let updated = match shown_at() {
        Some(at) => updated_label(at, chrono::Utc::now()),
        None => String::new(),
    };
    // A new board or filter starts at its first page. Page 1 has no `page`
    // pair, so nothing is pushed when the list is already there.
    let mut first_page = move || {
        unranked_page.set(1);
        if page.get() > 1 {
            page.go(1);
        }
    };
    let pick_board = move |next: &'static str| {
        if metric() != next {
            metric.set(next.to_string());
            first_page();
        }
    };

    rsx! {
        crate::styles::StyleOnce { id: STATS_VIEWS_CSS_ID, css: STATS_VIEWS_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-leaderboards-page-css", css: PAGE_CSS }
        PageShell { width: PageWidth::Wide, class: "lb-page",
            PageHeader {
                title: "Leaderboards",
                lead: PAGE_LEAD,
                meta: rsx! {
                    p { class: "lb-updated", "{updated}" }
                },
            }

            div { class: "lb-toolbar",
                BoardTabs { selected: metric(), on_pick: pick_board }
                div { class: "lb-filters", role: "group", aria_label: "Filters",
                    HeroSelect {
                        id: "leaderboard-hero".to_string(),
                        name: "leaderboard-hero".to_string(),
                        label: "Hero".to_string(),
                        value: hero(),
                        onchange: move |h| {
                            hero.set(h);
                            first_page();
                        },
                    }
                    SeasonSelect {
                        id: "leaderboard-season".to_string(),
                        name: "leaderboard-season".to_string(),
                        label: "Season".to_string(),
                        seasons: season.season_list(),
                        seasons_error: season.seasons_error(),
                        on_retry: move |_| season.retry(),
                        value: season.selected_id(),
                        onchange: move |s| {
                            season.choose(s);
                            first_page();
                        },
                    }
                }
            }

            BoardPanel {
                selected: metric(),
                board,
                view,
                message: message.unwrap_or_default(),
                hero_filtered,
                busy,
                page: page.get(),
                unranked_page: unranked_page(),
                on_retry: move |_| rows.restart(),
                on_page: move |next| {
                    page.go(next);
                    show_list_top(LB_PANEL);
                },
                on_unranked_page: move |next| unranked_page.set(next),
                on_all_heroes: move |_| {
                    hero.set(None);
                    first_page();
                },
                on_board: pick_board,
            }
        }
    }
}

/// The board tabs. A tablist, so it carries the shared `chip-row` and `chip`
/// classes itself: the shared `ChipRow` is a `role="group"` of pressed chips.
#[component]
fn BoardTabs(selected: String, on_pick: EventHandler<&'static str>) -> Element {
    rsx! {
        div { class: "chip-row", role: "tablist", aria_label: "Board",
            for (value , label) in BOARDS {
                TabButton {
                    key: "{value}",
                    class: "chip".to_string(),
                    label: label.to_string(),
                    id: metric_tab_id(value),
                    controls: LB_PANEL.to_string(),
                    selected: selected == value,
                    onclick: move |_| on_pick.call(value),
                }
            }
        }
    }
}

/// The tabpanel for one fetch state. The status line and both pagers are
/// always in the tree; only their text changes.
#[component]
fn BoardPanel(
    /// The selected tab. It names the panel.
    selected: String,
    /// The board the rows on screen were fetched for.
    board: String,
    view: BoardView,
    /// Error text or empty state title from [`leaderboard_message`].
    message: String,
    hero_filtered: bool,
    /// A fetch is running.
    busy: bool,
    page: usize,
    unranked_page: usize,
    on_retry: EventHandler<()>,
    on_page: EventHandler<usize>,
    on_unranked_page: EventHandler<usize>,
    on_all_heroes: EventHandler<()>,
    on_board: EventHandler<&'static str>,
) -> Element {
    let status = if busy || view == BoardView::Loading {
        LOADING
    } else {
        ""
    };
    let (ranked, unranked): (&[LeaderboardRow], &[LeaderboardRow]) = match &view {
        BoardView::Rows(split) => (&split.ranked, &split.unranked),
        _ => (&[], &[]),
    };
    // The pager clamps a page past the end; the rank numbers use the same page.
    let current = clamp_page(page, page_count(ranked.len(), BOARD_PAGE_SIZE));
    let first_rank = (current - 1) * BOARD_PAGE_SIZE + 1;
    let (ranked_plural, ranked_singular) = ranked_noun(ranked.len() + unranked.len());
    let label = board_label(&board);
    let note = board_note(&board);
    let sort = |metric: &str| (board == metric).then_some("descending");

    rsx! {
        section {
            id: LB_PANEL,
            class: "ui-stack pager-target",
            role: "tabpanel",
            aria_labelledby: metric_tab_id(&selected),
            tabindex: "-1",
            p { class: "lb-status", role: "status", "{status}" }
            {match &view {
                BoardView::Loading => rsx! {
                    SkeletonBars { rows: BOARD_PAGE_SIZE }
                },
                BoardView::Failed => rsx! {
                    LoadError {
                        message: message.clone(),
                        on_retry: move |_| on_retry.call(()),
                        retrying: busy,
                    }
                },
                BoardView::Empty if hero_filtered => rsx! {
                    EmptyCard {
                        title: message.clone(),
                        message: "Nobody has enough uploaded games on this hero. Try all heroes or another season.",
                        actions: rsx! {
                            button {
                                r#type: "button",
                                class: btn_class(true),
                                onclick: move |_| {
                                    on_all_heroes.call(());
                                    focus_element_by_id_soon(HERO_PICK_ID.to_string());
                                },
                                "Show all heroes"
                            }
                        },
                    }
                },
                BoardView::Empty => rsx! {
                    EmptyCard {
                        title: message.clone(),
                        message: "The board fills from personal stats uploaded with the tracker. Players show up after a few games.",
                        actions: rsx! {
                            Link { class: btn_class(true), to: Route::Stats {}, "Open your stats" }
                        },
                    }
                },
                BoardView::Rows(_) if ranked.is_empty() => rsx! {
                    EmptyCard {
                        title: WINRATE_NONE_RANKED,
                        message: "The record so far is listed under Too few games. The Games board ranks everyone.",
                        actions: rsx! {
                            button {
                                r#type: "button",
                                class: btn_class(true),
                                onclick: move |_| {
                                    on_board.call("games");
                                    // The button leaves with the board: focus
                                    // the Games tab it just picked.
                                    focus_element_by_id_soon(metric_tab_id("games"));
                                },
                                "See the Games board"
                            }
                        },
                    }
                },
                BoardView::Rows(_) => rsx! {
                    div {
                        class: "stats-table-wrap",
                        role: "region",
                        aria_label: "{label} ranking",
                        tabindex: "0",
                        table { class: "stats-table",
                            caption { class: "visually-hidden", "Players ranked by {label}" }
                            thead {
                                tr {
                                    th { class: "lb-rank", scope: "col",
                                        span { aria_hidden: "true", "#" }
                                        span { class: "visually-hidden", "Rank" }
                                    }
                                    th { scope: "col", "Player" }
                                    th { class: "num", scope: "col", aria_sort: sort("games"), "Games" }
                                    th { class: "rate", scope: "col", aria_sort: sort("winrate"), "Win rate" }
                                    th { class: "num", scope: "col", aria_sort: sort("kd"), "K/D" }
                                }
                            }
                            tbody {
                                for (i , r) in page_slice(ranked, current, BOARD_PAGE_SIZE).iter().enumerate() {
                                    BoardRow {
                                        key: "{r.member_id}",
                                        row: r.clone(),
                                        rank: first_rank + i,
                                        sorted: board.clone(),
                                    }
                                }
                            }
                        }
                    }
                    p { class: "lb-note", "{note}" }
                },
            }}
            Pager {
                id: "leaderboard-pager",
                page: current,
                total: ranked.len(),
                page_size: BOARD_PAGE_SIZE,
                noun: ranked_plural,
                noun_singular: ranked_singular,
                show_page_count: true,
                label: "Leaderboard pages",
                on_change: move |next| on_page.call(next),
            }
        }

        // The section is always in the tree so its pager's status line is
        // too; with no unranked players it is hidden and has nothing to say.
        section {
            class: "ui-stack",
            hidden: unranked.is_empty(),
            aria_labelledby: if !unranked.is_empty() { UNRANKED_TITLE_ID },
            if !unranked.is_empty() {
                h2 { id: UNRANKED_TITLE_ID, class: "ui-section-title", "{UNRANKED_TITLE}" }
                p { class: "lb-note", "{UNRANKED_NOTE}" }
                div {
                    class: "stats-table-wrap",
                    role: "region",
                    aria_labelledby: UNRANKED_TITLE_ID,
                    tabindex: "0",
                    table { class: "stats-table",
                        thead {
                            tr {
                                th { scope: "col", "Player" }
                                th { class: "num", scope: "col", "Games" }
                                th { class: "rate", scope: "col", "Record" }
                                th { class: "num", scope: "col", "K/D" }
                            }
                        }
                        tbody {
                            for r in page_slice(unranked, unranked_page, BOARD_PAGE_SIZE) {
                                BoardRow { key: "{r.member_id}", row: r.clone() }
                            }
                        }
                    }
                }
            }
            Pager {
                id: "leaderboard-unranked-pager",
                page: unranked_page,
                total: unranked.len(),
                page_size: BOARD_PAGE_SIZE,
                noun: "players with too few games",
                noun_singular: "player with too few games",
                show_page_count: true,
                label: "Too few games pages",
                on_change: move |next| on_unranked_page.call(next),
            }
        }
    }
}

/// One player. With a `rank` it is a row of the ranked table; without one it
/// is a row of the unranked list, which has no rank column.
#[component]
fn BoardRow(
    row: LeaderboardRow,
    #[props(default)] rank: Option<usize>,
    /// The board this ranked row is sorted by. Its number cell takes body
    /// ink. Empty in the unranked list, which is not a ranking.
    #[props(default)]
    sorted: String,
) -> Element {
    let wr = wr_cell(&row);
    let kd = format!("{:.2}", row.kd);
    let num_class = |metric: &str| {
        if sorted == metric {
            "num is-sorted"
        } else {
            "num"
        }
    };
    rsx! {
        tr {
            if let Some(rank) = rank {
                td { class: "lb-rank", "{rank}" }
            }
            td { class: "lb-player",
                Link {
                    to: Route::MemberProfile { id: row.member_id.clone() },
                    "{row.display_name}"
                }
            }
            td { class: num_class("games"), "{row.games}" }
            td { class: "rate",
                if let Some(pct) = rate_bar_pct(&wr) {
                    div { class: "stats-rate",
                        span { class: "lb-rate-val", "{wr.text}" }
                        span { class: "stats-rate-bar", aria_hidden: "true",
                            span { class: "stats-rate-fill", style: "width:{pct}%;" }
                        }
                    }
                } else {
                    span { class: if wr.muted { "lb-record is-muted" } else { "lb-record" }, "{wr.text}" }
                }
            }
            td { class: num_class("kd"), "{kd}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn at(secs_from_now: i64, now: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
        now + chrono::Duration::seconds(secs_from_now)
    }

    #[test]
    fn filter_controls_pass_stable_id_and_name() {
        let src = include_str!("leaderboards.rs");
        for id in ["leaderboard-hero", "leaderboard-season"] {
            assert!(
                src.contains(&format!("id: \"{id}\".to_string()")),
                "{id} id"
            );
            assert!(
                src.contains(&format!("name: \"{id}\".to_string()")),
                "{id} name"
            );
        }
    }

    #[test]
    fn updated_label_just_now_seconds_and_minutes() {
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap();
        assert_eq!(updated_label(at(0, now), now), "Updated just now");
        assert_eq!(updated_label(at(-4, now), now), "Updated just now");
        assert_eq!(updated_label(at(-5, now), now), "Updated 5s ago");
        assert_eq!(updated_label(at(-59, now), now), "Updated 59s ago");
        assert_eq!(updated_label(at(-60, now), now), "Updated 1m ago");
        assert_eq!(updated_label(at(-125, now), now), "Updated 2m ago");
    }

    #[test]
    fn updated_label_hours_start_at_sixty_minutes() {
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap();
        assert_eq!(updated_label(at(-59 * 60, now), now), "Updated 59m ago");
        assert_eq!(updated_label(at(-60 * 60, now), now), "Updated 1h ago");
        assert_eq!(updated_label(at(-2 * 3600, now), now), "Updated 2h ago");
        assert_eq!(updated_label(at(-5 * 3600, now), now), "Updated 5h ago");
    }

    #[test]
    fn updated_label_future_stamp_reads_as_just_now() {
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap();
        assert_eq!(updated_label(at(30, now), now), "Updated just now");
    }

    #[test]
    fn parse_cached_at_hides_malformed_stamps() {
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap();
        assert_eq!(parse_cached_at(None), None);
        assert_eq!(parse_cached_at(Some("")), None);
        assert_eq!(parse_cached_at(Some("   ")), None);
        assert_eq!(parse_cached_at(Some("not-a-timestamp")), None);
        assert_eq!(parse_cached_at(Some("2026-13-99")), None);
        let raw = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        assert_eq!(parse_cached_at(Some(&raw)), Some(now));
    }

    #[test]
    fn payload_without_cached_at_still_parses() {
        let payload: LeaderboardPayload = serde_json::from_str(
            r#"{"rows":[{"member_id":"m","display_name":"M","games":1,"winrate":1.0,"kd":1.0}]}"#,
        )
        .unwrap();
        assert!(payload.cached_at.is_none());
        assert_eq!(payload.rows.len(), 1);
        assert_eq!(parse_cached_at(payload.cached_at.as_deref()), None);
    }

    #[test]
    fn unresolved_season_path_holds_instead_of_failing() {
        assert_eq!(
            leaderboard_hold(""),
            Some(LeaderboardLoad::Hold),
            "an empty season path is the unresolved hold, not a failure"
        );
        assert_eq!(
            leaderboard_hold("/api/public/leaderboards?metric=winrate"),
            None
        );
    }

    fn row(games: u32, wins: u32, losses: u32, draws: u32, winrate: f32) -> LeaderboardRow {
        LeaderboardRow {
            member_id: "m".into(),
            display_name: "M".into(),
            games,
            wins,
            losses,
            draws,
            winrate,
            kd: 1.0,
        }
    }

    #[test]
    fn wr_cell_follows_the_small_sample_rule() {
        let two = wr_cell(&row(2, 2, 0, 0, 1.0));
        assert_eq!(two.text, "2W-0L");
        assert!(two.muted);

        assert_eq!(rate_bar_pct(&two), None);

        let six = wr_cell(&row(6, 3, 2, 1, 0.5));
        assert_eq!(six.text, "3W-2L-1D");
        assert!(!six.muted);
        // A record has no bar. Only a win rate gets one.
        assert_eq!(rate_bar_pct(&six), None);

        let ten = wr_cell(&row(10, 8, 2, 0, 0.8));
        assert_eq!(ten.text, "80%");
        assert!(!ten.muted);
        assert_eq!(rate_bar_pct(&ten), Some(80));
    }

    #[test]
    fn leaderboard_row_reads_the_record_and_older_rows() {
        let new: LeaderboardRow = serde_json::from_str(
            r#"{"member_id":"m","display_name":"M","games":9,"wins":5,"losses":3,"draws":1,"winrate":0.5555556,"kd":1.2}"#,
        )
        .unwrap();
        assert_eq!(wr_cell(&new).text, "5W-3L-1D");

        let old: LeaderboardRow = serde_json::from_str(
            r#"{"member_id":"m","display_name":"M","games":6,"winrate":0.5,"kd":1.0}"#,
        )
        .unwrap();
        assert_eq!((old.wins, old.losses, old.draws), (0, 0, 0));
        assert_eq!(wr_cell(&old).text, "3W");
    }

    fn named(id: &str, name: &str, games: u32, wins: u32, winrate: f32) -> LeaderboardRow {
        LeaderboardRow {
            member_id: id.into(),
            display_name: name.into(),
            games,
            wins,
            losses: games - wins,
            draws: 0,
            winrate,
            kd: 1.0,
        }
    }

    fn ids(rows: &[LeaderboardRow]) -> Vec<&str> {
        rows.iter().map(|r| r.member_id.as_str()).collect()
    }

    #[test]
    fn winrate_board_ranks_from_ten_games_only() {
        let rows = vec![
            named("nine", "Nine", 9, 9, 1.0),
            named("ten", "Ten", 10, 6, 0.6),
        ];
        let split = board_split("winrate", &rows);
        assert_eq!(ids(&split.ranked), vec!["ten"]);
        assert_eq!(ids(&split.unranked), vec!["nine"]);
        assert_eq!(wr_cell(&split.ranked[0]).text, "60%");
        assert_eq!(wr_cell(&split.unranked[0]).text, "9W-0L");
    }

    #[test]
    fn winrate_board_ranks_by_decided_games() {
        let mut open = named("open", "Open", 14, 6, 6.0 / 14.0);
        open.losses = 3;
        let mut ten = named("ten", "Ten", 12, 6, 0.5);
        ten.losses = 3;
        ten.draws = 1;
        let old: LeaderboardRow = serde_json::from_str(
            r#"{"member_id":"old","display_name":"Old","games":10,"winrate":0.6,"kd":1.0}"#,
        )
        .unwrap();
        let split = board_split("winrate", &[open, ten, old]);
        assert_eq!(ids(&split.ranked), vec!["ten", "old"]);
        assert_eq!(ids(&split.unranked), vec!["open"]);
        assert_eq!(wr_cell(&split.unranked[0]).text, "6W-3L");
        assert!(!wr_cell(&split.unranked[0]).is_rate);
        assert_eq!(wr_cell(&split.ranked[0]).text, "50%");
        assert_eq!(wr_cell(&split.ranked[1]).text, "60%");
        assert!(split.ranked.iter().all(|r| wr_cell(r).is_rate));
        assert_eq!(UNRANKED_TITLE, "Too few games");
    }

    #[test]
    fn winrate_board_keeps_server_order_for_ranked_rows() {
        let rows = vec![
            named("a", "A", 12, 9, 0.75),
            named("short", "Short", 5, 5, 1.0),
            named("b", "B", 20, 12, 0.6),
            named("c", "C", 10, 5, 0.5),
        ];
        let split = board_split("winrate", &rows);
        assert_eq!(ids(&split.ranked), vec!["a", "b", "c"]);
        assert_eq!(ids(&split.unranked), vec!["short"]);
    }

    #[test]
    fn winrate_board_with_ten_plus_only_has_an_empty_unranked_list() {
        let rows = vec![named("a", "A", 10, 7, 0.7), named("b", "B", 31, 15, 0.48)];
        let split = board_split("winrate", &rows);
        assert_eq!(ids(&split.ranked), vec!["a", "b"]);
        assert!(split.unranked.is_empty());
        assert!(board_split("winrate", &[]).unranked.is_empty());
    }

    #[test]
    fn unranked_list_sorts_by_games_then_name() {
        let rows = vec![
            named("m1", "zed", 6, 3, 0.5),
            named("m2", "Amy", 9, 4, 0.44),
            named("m3", "bob", 6, 2, 0.33),
            named("m4", "Cat", 5, 5, 1.0),
            named("m5", "Ann", 9, 6, 0.67),
        ];
        let split = board_split("winrate", &rows);
        assert!(split.ranked.is_empty());
        assert_eq!(ids(&split.unranked), vec!["m2", "m5", "m3", "m1", "m4"]);
    }

    #[test]
    fn unranked_rows_show_the_record_or_wins_from_an_older_server() {
        let mut drawn = named("d", "D", 9, 5, 5.0 / 9.0);
        drawn.losses = 3;
        drawn.draws = 1;
        let old: LeaderboardRow = serde_json::from_str(
            r#"{"member_id":"o","display_name":"O","games":7,"winrate":0.5714286,"kd":1.0}"#,
        )
        .unwrap();
        let split = board_split("winrate", &[drawn, old]);
        let texts: Vec<String> = split.unranked.iter().map(|r| wr_cell(r).text).collect();
        assert_eq!(texts, vec!["5W-3L-1D", "4W"]);
        assert!(split.unranked.iter().all(|r| !wr_cell(r).is_rate));
    }

    #[test]
    fn kd_and_games_boards_are_not_split() {
        let rows = vec![named("a", "A", 5, 3, 0.6), named("b", "B", 12, 6, 0.5)];
        for metric in ["kd", "games"] {
            let split = board_split(metric, &rows);
            assert_eq!(ids(&split.ranked), vec!["a", "b"], "{metric}");
            assert!(split.unranked.is_empty(), "{metric}");
            assert_eq!(board_note(metric), LIST_RULE_NOTE);
        }
        assert_eq!(board_note("winrate"), WINRATE_BOARD_NOTE);
    }

    #[test]
    fn winrate_copy_has_no_long_dashes() {
        for text in [
            WINRATE_BOARD_NOTE,
            WINRATE_NONE_RANKED,
            UNRANKED_TITLE,
            UNRANKED_NOTE,
        ] {
            assert!(
                !text.contains('\u{2014}') && !text.contains('\u{2013}'),
                "{text}"
            );
        }
    }

    #[test]
    fn hold_renders_loading_not_the_failure() {
        assert_eq!(leaderboard_message(None, false), Some(LOADING));
        assert_eq!(
            leaderboard_message(Some(&LeaderboardLoad::Hold), false),
            Some(LOADING)
        );
        assert_ne!(
            leaderboard_message(Some(&LeaderboardLoad::Hold), false),
            Some(LOAD_FAILED)
        );
        assert_eq!(
            leaderboard_message(Some(&LeaderboardLoad::Failed), false),
            Some(LOAD_FAILED)
        );
        assert_eq!(
            leaderboard_message(Some(&LeaderboardLoad::Rows(Vec::new())), true),
            Some("No ranked players on this hero yet")
        );
        assert_eq!(
            leaderboard_message(Some(&LeaderboardLoad::Rows(Vec::new())), false),
            Some("No ranked players yet")
        );
        let one = LeaderboardLoad::Rows(vec![named("m1", "Player01", 1, 1, 1.0)]);
        assert_eq!(leaderboard_message(Some(&one), false), None);

        // The panel state agrees with the words.
        assert_eq!(board_view(None), BoardView::Loading);
        let slot = |load| ("winrate".to_string(), load);
        assert_eq!(
            board_view(Some(&slot(LeaderboardLoad::Hold))),
            BoardView::Loading
        );
        assert_eq!(
            board_view(Some(&slot(LeaderboardLoad::Failed))),
            BoardView::Failed
        );
        assert_eq!(
            board_view(Some(&slot(LeaderboardLoad::Rows(Vec::new())))),
            BoardView::Empty
        );
        assert!(matches!(
            board_view(Some(&slot(one))),
            BoardView::Rows(split) if split.ranked.is_empty() && split.unranked.len() == 1
        ));
    }

    #[test]
    fn metric_tab_ids_are_unique_and_differ_from_the_panel() {
        let ids: Vec<String> = ["winrate", "kd", "games"]
            .into_iter()
            .map(metric_tab_id)
            .collect();
        for (i, id) in ids.iter().enumerate() {
            assert!(!ids[i + 1..].contains(id), "{id}");
            assert_ne!(id, LB_PANEL);
        }
    }

    // ---- markup ----

    use crate::components::ui::test_support::{
        assert_sizes_from_tokens, css_rule, has_decl, render_routed,
    };

    /// `count` ranked players, best first: Player01, Player02, ...
    fn players(count: usize) -> Vec<LeaderboardRow> {
        (1..=count)
            .map(|n| {
                let mut row = named(
                    &format!("member:{n:02}"),
                    &format!("Player{n:02}"),
                    20,
                    15 - (n as u32).min(15),
                    0.0,
                );
                row.winrate = row.wins as f32 / row.games as f32;
                row
            })
            .collect()
    }

    fn rows_view(board: &str, rows: &[LeaderboardRow]) -> BoardView {
        board_view(Some(&(
            board.to_string(),
            LeaderboardLoad::Rows(rows.to_vec()),
        )))
    }

    #[derive(Clone)]
    struct PanelCase {
        board: &'static str,
        view: BoardView,
        hero_filtered: bool,
        busy: bool,
        page: usize,
    }

    impl PanelCase {
        fn new(board: &'static str, view: BoardView) -> Self {
            PanelCase {
                board,
                view,
                hero_filtered: false,
                busy: false,
                page: 1,
            }
        }
    }

    /// The page chrome around the panel, without the two pickers: they need
    /// the season fetch.
    fn render_case(case: PanelCase) -> String {
        render_routed(move || {
            let case = case.clone();
            let message = match &case.view {
                BoardView::Loading => LOADING,
                BoardView::Failed => LOAD_FAILED,
                BoardView::Empty if case.hero_filtered => NO_PLAYERS_ON_HERO,
                BoardView::Empty => NO_PLAYERS,
                BoardView::Rows(_) => "",
            };
            rsx! {
                PageShell { width: PageWidth::Wide, class: "lb-page",
                    PageHeader { title: "Leaderboards", lead: PAGE_LEAD }
                    BoardTabs { selected: case.board.to_string(), on_pick: move |_| {} }
                    BoardPanel {
                        selected: case.board.to_string(),
                        board: case.board.to_string(),
                        view: case.view,
                        message,
                        hero_filtered: case.hero_filtered,
                        busy: case.busy,
                        page: case.page,
                        unranked_page: 1,
                        on_retry: move |_| {},
                        on_page: move |_| {},
                        on_unranked_page: move |_| {},
                        on_all_heroes: move |_| {},
                        on_board: move |_| {},
                    }
                }
            }
        })
    }

    fn all_states() -> Vec<(&'static str, String)> {
        let mut hero = PanelCase::new("winrate", BoardView::Empty);
        hero.hero_filtered = true;
        vec![
            (
                "loading",
                render_case(PanelCase::new("winrate", BoardView::Loading)),
            ),
            (
                "failed",
                render_case(PanelCase::new("winrate", BoardView::Failed)),
            ),
            (
                "empty",
                render_case(PanelCase::new("winrate", BoardView::Empty)),
            ),
            ("empty hero", render_case(hero)),
            (
                "rows",
                render_case(PanelCase::new(
                    "winrate",
                    rows_view("winrate", &players(12)),
                )),
            ),
        ]
    }

    #[test]
    fn every_state_has_one_h1_no_main_and_its_live_regions() {
        for (name, html) in all_states() {
            assert_eq!(html.matches("<h1").count(), 1, "{name}: {html}");
            assert!(html.contains("<h1>Leaderboards</h1>"), "{name}: {html}");
            assert!(!html.contains("<main"), "{name}: {html}");
            assert!(
                html.contains("class=\"ui-shell ui-shell--wide lb-page\""),
                "{name}: {html}"
            );
            // The status line and both pager lines exist before their text does.
            assert_eq!(
                html.matches("<p class=\"lb-status\" role=\"status\">")
                    .count(),
                1,
                "{name}: {html}"
            );
            for id in ["leaderboard-pager-meta", "leaderboard-unranked-pager-meta"] {
                assert_eq!(
                    html.matches(&format!("id=\"{id}\"")).count(),
                    1,
                    "{name}: {id}"
                );
            }
        }
        let src = include_str!("leaderboards.rs");
        let page = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(!page.contains("main {"), "the shell owns <main>");
    }

    /// The real page, not the hand-built shell above: one H1, no main, the
    /// panel, and the controls focus moves to when a button removes itself.
    #[test]
    fn the_real_page_has_one_h1_and_the_focus_targets() {
        let html = render_routed(|| rsx! { Leaderboards {} });
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Leaderboards</h1>"), "{html}");
        assert!(!html.contains("<main"), "{html}");
        assert!(html.contains(&format!("id=\"{LB_PANEL}\"")), "{html}");
        assert!(html.contains(&format!("id=\"{HERO_PICK_ID}\"")), "{html}");
        assert!(
            html.contains(&format!("id=\"{}\"", metric_tab_id("games"))),
            "{html}"
        );
    }

    #[test]
    fn a_full_fetch_counts_the_players_shown_and_the_short_list_owns_its_pager() {
        assert_eq!(ranked_noun(BOARD_LIMIT - 1).0, "ranked players");
        assert_eq!(ranked_noun(BOARD_LIMIT).0, "ranked players shown");
        assert!(include_str!("leaderboards.rs").contains("&limit={BOARD_LIMIT}"));
        let full = render_case(PanelCase::new("games", rows_view("games", &players(BOARD_LIMIT))));
        assert!(full.contains("of 50 ranked players shown"), "{full}");
        // The Too few games pager sits inside its own section.
        let mut rows = players(3);
        rows.push(named("member:99", "Player99", 2, 1, 0.5));
        let html = render_case(PanelCase::new("winrate", rows_view("winrate", &rows)));
        let title = html
            .find(&format!("id=\"{UNRANKED_TITLE_ID}\""))
            .expect("short list");
        let section = html[..title].rfind("<section").expect("section");
        let pager = html.find("id=\"leaderboard-unranked-pager-meta\"").expect("pager");
        let close = section + html[section..].find("</section>").unwrap();
        assert!(section < pager && pager < close, "{html}");
        // The heading copy matches the page it opens.
        let empty = render_case(PanelCase::new("winrate", BoardView::Empty));
        assert!(!empty.contains("My Stats"), "{empty}");
    }

    #[test]
    fn buttons_that_remove_themselves_hand_focus_on() {
        let src = include_str!("leaderboards.rs");
        let page = &src[..src.find("#[cfg(test)]").unwrap()];
        for button in ["\"Show all heroes\"", "\"See the Games board\""] {
            let end = page.find(button).expect(button);
            let handler = &page[page[..end].rfind("onclick:").expect(button)..end];
            assert!(handler.contains("focus_element_by_id_soon("), "{button}: {handler}");
        }
        // The empty card follows the rows on screen, not the filter that is
        // still fetching.
        assert!(page.contains("let hero_filtered = shown_for_hero();"));
        assert!(!page.contains("let hero_filtered = hero().is_some();"));
    }

    #[test]
    fn tabs_are_a_named_tablist_that_labels_the_panel() {
        let html = render_case(PanelCase::new("kd", rows_view("kd", &players(3))));
        assert!(
            html.contains("<div class=\"chip-row\" role=\"tablist\" aria-label=\"Board\">"),
            "{html}"
        );
        assert_eq!(html.matches("role=\"tab\"").count(), 3, "{html}");
        assert_eq!(html.matches("aria-selected=\"true\"").count(), 1, "{html}");
        assert!(
            html.contains("id=\"leaderboard-tab-kd\" aria-controls=\"leaderboard-panel\" aria-selected=\"true\" class=\"chip active\""),
            "{html}"
        );
        assert!(
            html.contains("<section id=\"leaderboard-panel\" class=\"ui-stack pager-target\" role=\"tabpanel\" aria-labelledby=\"leaderboard-tab-kd\" tabindex=\"-1\">"),
            "{html}"
        );
        // The ranked column says so without colour.
        assert!(
            html.contains("<th class=\"num\" scope=\"col\" aria-sort=\"descending\">K/D</th>"),
            "{html}"
        );
        assert_eq!(html.matches("aria-sort").count(), 1, "{html}");
    }

    /// Design pass bug: the Win rate header kept body ink on the K/D and
    /// Games boards. The ink now follows the sorted column, header and cells.
    #[test]
    fn sorted_column_ink_follows_the_board() {
        let rule = css_rule(PAGE_CSS, ".lb-page .stats-table tbody td.num.is-sorted {");
        assert!(has_decl(&rule, "color: var(--table-row-fg)"), "{rule}");
        assert!(PAGE_CSS.contains(".lb-page .stats-table thead th[aria-sort],"));
        assert!(!PAGE_CSS.contains("th.rate"), "no fixed Win rate highlight");

        let kd = render_case(PanelCase::new("kd", rows_view("kd", &players(3))));
        assert_eq!(
            kd.matches("<td class=\"num is-sorted\">").count(),
            3,
            "{kd}"
        );
        assert!(kd.contains("<td class=\"num is-sorted\">1.00</td>"), "{kd}");
        assert!(kd.contains("<td class=\"num\">20</td>"), "{kd}");

        let games = render_case(PanelCase::new("games", rows_view("games", &players(3))));
        assert!(
            games.contains("<td class=\"num is-sorted\">20</td>"),
            "{games}"
        );
        assert!(games.contains("<td class=\"num\">1.00</td>"), "{games}");

        // Win rate board: the rate column is the sorted one, so both
        // number columns stay muted, as in the frame.
        let wr = render_case(PanelCase::new("winrate", rows_view("winrate", &players(3))));
        assert!(!wr.contains("is-sorted"), "{wr}");
    }

    /// Design pass bug: at 390 the Hero and Season pickers each took a row.
    /// The phone frame has both on one row.
    #[test]
    fn phone_pickers_share_one_row() {
        let phone = &PAGE_CSS[PAGE_CSS.find("@media (max-width: 820px)").unwrap()..];
        let filters = css_rule(phone, ".lb-filters {");
        assert!(has_decl(&filters, "display: grid"), "{filters}");
        assert!(
            has_decl(
                &filters,
                "grid-template-columns: repeat(2, minmax(0, max-content))"
            ),
            "{filters}"
        );
        let select = css_rule(phone, ".lb-filters select.ui-field {");
        assert!(has_decl(&select, "flex: 0 1 auto"), "{select}");
        let label = css_rule(phone, ".lb-filters .ui-label {");
        assert!(has_decl(&label, "flex: none"), "{label}");
    }

    #[test]
    fn loading_is_labelled_and_failure_offers_retry() {
        let loading = render_case(PanelCase::new("winrate", BoardView::Loading));
        assert!(
            loading.contains("<p class=\"lb-status\" role=\"status\">Loading leaderboard</p>"),
            "{loading}"
        );
        assert!(loading.contains("ui-skel-row"), "{loading}");
        assert!(!loading.contains("<table"), "{loading}");

        let failed = render_case(PanelCase::new("winrate", BoardView::Failed));
        assert!(failed.contains("role=\"alert\""), "{failed}");
        assert!(
            failed.contains("load the leaderboard. Check your connection"),
            "{failed}"
        );
        assert!(failed.contains(">Retry</button>"), "{failed}");
        assert!(
            failed.contains("<p class=\"lb-status\" role=\"status\"></p>"),
            "{failed}"
        );

        // A retry keeps the error and its button, and says it is loading.
        let mut retry = PanelCase::new("winrate", BoardView::Failed);
        retry.busy = true;
        let retrying = render_case(retry);
        assert!(
            retrying.contains("<p class=\"lb-status\" role=\"status\">Loading leaderboard</p>"),
            "{retrying}"
        );
        assert!(retrying.contains(">Retrying...</button>"), "{retrying}");
        assert!(retrying.contains("aria-disabled=\"true\""), "{retrying}");

        // A filter change keeps the previous rows and says it is loading.
        let mut refetch = PanelCase::new("winrate", rows_view("winrate", &players(3)));
        refetch.busy = true;
        let refetching = render_case(refetch);
        assert!(
            refetching.contains(">Loading leaderboard</p>"),
            "{refetching}"
        );
        assert!(refetching.contains(">Player01</a>"), "{refetching}");
    }

    #[test]
    fn empty_states_say_what_is_missing_and_offer_a_next_step() {
        let empty = render_case(PanelCase::new("winrate", BoardView::Empty));
        assert!(empty.contains("<h2>No ranked players yet</h2>"), "{empty}");
        assert!(
            empty.contains("class=\"btn is-primary\"") && empty.contains(">Open your stats</a>"),
            "{empty}"
        );
        assert!(empty.contains("href=\"/stats\""), "{empty}");

        let mut hero = PanelCase::new("winrate", BoardView::Empty);
        hero.hero_filtered = true;
        let hero = render_case(hero);
        assert!(
            hero.contains("<h2>No ranked players on this hero yet</h2>"),
            "{hero}"
        );
        assert!(hero.contains(">Show all heroes</button>"), "{hero}");

        // Rows, but nobody at 10 decided games: the record list still shows.
        let short = vec![named("member:01", "Player01", 6, 3, 0.5)];
        let none = render_case(PanelCase::new("winrate", rows_view("winrate", &short)));
        assert!(
            none.contains("<h2>Nobody has 10 decided games yet</h2>"),
            "{none}"
        );
        assert!(none.contains(">See the Games board</button>"), "{none}");
        assert!(
            none.contains(
                "<h2 id=\"leaderboard-unranked-title\" class=\"ui-section-title\">Too few games</h2>"
            ),
            "{none}"
        );
        assert!(none.contains(">3W-3L</span>"), "{none}");
        assert!(none.contains(">1 player with too few games</p>"), "{none}");
        // No button sits inside a link.
        for html in [&empty, &hero, &none] {
            for link in html.split("<a ").skip(1) {
                let inner = link.split("</a>").next().unwrap_or("");
                assert!(!inner.contains("<button"), "{inner}");
            }
        }
    }

    #[test]
    fn ranked_rows_are_paged_eight_at_a_time_with_running_ranks() {
        let rows = players(12);
        let first = render_case(PanelCase::new("winrate", rows_view("winrate", &rows)));
        assert_eq!(
            first.matches("<td class=\"lb-rank\">").count(),
            8,
            "{first}"
        );
        assert!(
            first.contains(">1 to 8 of 12 ranked players · Page 1 of 2</p>"),
            "{first}"
        );
        assert!(first.contains("<td class=\"lb-rank\">1</td>"), "{first}");
        assert!(first.contains(">Player08</a>"), "{first}");
        assert!(!first.contains(">Player09</a>"), "{first}");
        assert!(
            first.contains("<nav class=\"pager-pages\" aria-label=\"Leaderboard pages\">"),
            "{first}"
        );

        let mut second = PanelCase::new("winrate", rows_view("winrate", &rows));
        second.page = 2;
        let second = render_case(second);
        assert_eq!(
            second.matches("<td class=\"lb-rank\">").count(),
            4,
            "{second}"
        );
        // Ranks run on from the first page.
        assert!(second.contains("<td class=\"lb-rank\">9</td>"), "{second}");
        assert!(second.contains("<td class=\"lb-rank\">12</td>"), "{second}");
        assert!(!second.contains("<td class=\"lb-rank\">1</td>"), "{second}");

        // A page past the end shows the last page, with the last page's ranks.
        let mut past = PanelCase::new("winrate", rows_view("winrate", &rows));
        past.page = 9;
        let past = render_case(past);
        assert!(past.contains("<td class=\"lb-rank\">9</td>"), "{past}");
        assert!(past.contains("Page 2 of 2</p>"), "{past}");
    }

    #[test]
    fn rows_show_display_names_and_a_rate_in_words_with_a_hidden_bar() {
        let rows = vec![
            named("member:abc123", "Player01", 20, 13, 0.65),
            named("member:def456", "Player02", 20, 8, 0.4),
        ];
        let html = render_case(PanelCase::new("winrate", rows_view("winrate", &rows)));
        assert!(html.contains(">Player01</a>"), "{html}");
        assert!(html.contains(">Player02</a>"), "{html}");
        // The raw id is only ever inside the link address.
        assert!(!html.contains(">member:abc123<"), "{html}");
        // The number is text; the bar repeats it and is hidden from readers.
        assert!(
            html.contains(
                "<span class=\"lb-rate-val\">65%</span><span class=\"stats-rate-bar\" aria-hidden=\"true\"><span class=\"stats-rate-fill\" style=\"width:65%;\"></span></span>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"lb-rate-val\">40%</span>"),
            "{html}"
        );
        // A low rate is not painted: one bar ink, no win or loss class.
        for word in ["var(--win)", "var(--loss)", "is-win", "is-loss"] {
            assert!(!PAGE_CSS.contains(word), "{word}");
            assert!(!html.contains(word), "{word}");
        }
        assert!(html.contains("<td class=\"num\">20</td>"), "{html}");
        assert!(html.contains("<td class=\"num\">1.00</td>"), "{html}");
        // The scroll container can take focus and has a name.
        assert!(
            html.contains(
                "<div class=\"stats-table-wrap\" role=\"region\" aria-label=\"Win rate ranking\" tabindex=\"0\">"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<caption class=\"visually-hidden\">Players ranked by Win rate</caption>"
            ),
            "{html}"
        );
        assert!(html.contains(WINRATE_BOARD_NOTE), "{html}");
    }

    #[test]
    fn element_ids_are_fixed_names_or_come_from_data() {
        let html = render_case(PanelCase::new(
            "winrate",
            rows_view("winrate", &players(12)),
        ));
        let mut ids: Vec<&str> = html
            .split(" id=\"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "duplicate id in {ids:?}");
        for (value, _) in BOARDS {
            assert!(ids.contains(&metric_tab_id(value).as_str()), "{value}");
        }
    }

    #[test]
    fn page_css_uses_tokens_and_keeps_the_page_from_scrolling_sideways() {
        assert_sizes_from_tokens(PAGE_CSS);
        // The table is the shared stats table: its wrap is the sideways
        // scroll container and its rows follow the density setting.
        let wrap = css_rule(STATS_VIEWS_CSS, ".stats-table-wrap {");
        assert!(has_decl(&wrap, "overflow-x: auto"), "{wrap}");
        assert!(has_decl(&wrap, "max-width: 100%"), "{wrap}");
        let cell = css_rule(STATS_VIEWS_CSS, ".stats-table tbody td {");
        assert!(has_decl(&cell, "height: var(--table-row-height)"), "{cell}");
        // The player link is a 44px target.
        let link = css_rule(PAGE_CSS, ".lb-page .stats-table .lb-player a {");
        assert!(
            has_decl(&link, "min-height: var(--hit-target-min)"),
            "{link}"
        );
        assert!(
            has_decl(&link, "min-width: var(--hit-target-min)"),
            "{link}"
        );
        let select = css_rule(PAGE_CSS, ".lb-filters select.ui-field {");
        assert!(
            has_decl(&select, "min-height: var(--hit-target-min)"),
            "{select}"
        );
        // The empty status line leaves the layout but stays in the tree.
        let empty = css_rule(PAGE_CSS, ".lb-status:empty {");
        assert!(has_decl(&empty, "position: absolute"), "{empty}");
        assert!(!empty.contains("display"), "{empty}");
        // Phone: the toolbar stacks.
        let phone = &PAGE_CSS[PAGE_CSS.find("@media (max-width: 820px)").unwrap()..];
        let toolbar = css_rule(phone, ".lb-toolbar {");
        assert!(has_decl(&toolbar, "flex-direction: column"), "{toolbar}");
    }

    #[test]
    fn page_copy_has_no_long_dashes_or_wrong_names() {
        let src = include_str!("leaderboards.rs");
        let page = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(
            !page.contains('\u{2014}') && !page.contains('\u{2013}'),
            "long dash in the page source"
        );
        assert!(!page.contains(&format!("Overwatch {}", 2)));
    }
}
