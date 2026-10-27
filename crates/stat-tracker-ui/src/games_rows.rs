//! Pure layout and labels for the 0.6 Games table (variant A, top tabs):
//! which columns fit, the sections (sessions with their headers, or one
//! flat list), the form strip, result labels and row marks, and the sort
//! (ported from #555). No iced types here, so every rule has a unit test;
//! [`crate::games_table`] draws it.

use std::cmp::Ordering;
use std::collections::HashSet;

use chrono::{DateTime, Local, NaiveDate, TimeZone, Timelike, Utc};

use crate::model::{Game, Outcome, game_card_in_progress};

/// Window width from which the wide grid applies (spec: 1440 px).
pub const WIDE_FROM: f32 = 1440.0;
/// Window width under which Length, Crew, E/D/A and Hero are hidden
/// (spec: 1100 px).
pub const NARROW_UNDER: f32 = 1100.0;

/// Row height in px.
pub const ROW_H: f32 = 40.0;
/// Row height with Compact rows on.
pub const ROW_H_COMPACT: f32 = 32.0;
/// Column header height in px.
pub const HEADER_H: f32 = 32.0;
/// At most this many rows are drawn; the note under the list says so.
pub const ROW_CAP: usize = 500;

/// Which grid the table uses at a window width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Grid {
    /// 1440 px windows and up.
    #[default]
    Wide,
    /// 1100 to 1439 px windows.
    Mid,
    /// Under 1100 px: Length, Crew, E/D/A and Hero are hidden.
    Narrow,
}

impl Grid {
    /// The grid at a window width. The window, not the content: the page
    /// gutter is 24 px a side under 1440 and 32 px from it, so one content
    /// width can stand for windows on both sides of the switch.
    pub fn for_window(window_width: f32) -> Self {
        if window_width >= WIDE_FROM {
            Grid::Wide
        } else if window_width >= NARROW_UNDER {
            Grid::Mid
        } else {
            Grid::Narrow
        }
    }
}

/// One table column, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Col {
    Stripe,
    Result,
    Map,
    Hero,
    Eda,
    Rank,
    Length,
    Time,
    Crew,
    Unsure,
}

/// A column's width: fixed px or a share of what is left (the spec's
/// `minmax(Npx, Kfr)`, with `K` in tenths).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Width {
    Px(f32),
    Share(u16),
}

impl Col {
    /// Header label. The stripe and the unsure chip have none.
    pub fn label(self) -> &'static str {
        match self {
            Col::Stripe | Col::Unsure => "",
            Col::Result => "RESULT",
            Col::Map => "MAP",
            Col::Hero => "HERO",
            Col::Eda => "E/D/A",
            Col::Rank => "RANK",
            Col::Length => "LENGTH",
            Col::Time => "TIME",
            Col::Crew => "CREW",
        }
    }

    /// Columns whose header sorts. Rank, Length and Crew hold no data yet.
    pub fn sortable(self) -> bool {
        matches!(
            self,
            Col::Result | Col::Map | Col::Hero | Col::Eda | Col::Time
        )
    }

    /// The direction a first click on this header sorts: newest, most
    /// eliminations and wins first; names A to Z.
    pub fn first_desc(self) -> bool {
        matches!(self, Col::Time | Col::Eda | Col::Result)
    }
}

/// The columns at `grid` with their widths (spec r1.1 `A-games.html`).
/// The header row and every game row draw from this one list, so a column
/// the rows fold away at 1024 (Hero, E/D/A) is gone from the header too.
pub fn columns(grid: Grid) -> Vec<(Col, Width)> {
    use Width::{Px, Share};
    match grid {
        Grid::Wide => vec![
            (Col::Stripe, Px(4.0)),
            (Col::Result, Px(88.0)),
            (Col::Map, Share(12)),
            (Col::Hero, Share(13)),
            (Col::Eda, Px(88.0)),
            (Col::Rank, Px(72.0)),
            (Col::Length, Px(56.0)),
            (Col::Time, Px(56.0)),
            (Col::Crew, Px(72.0)),
            (Col::Unsure, Px(156.0)),
        ],
        Grid::Mid => vec![
            (Col::Stripe, Px(4.0)),
            (Col::Result, Px(80.0)),
            (Col::Map, Share(12)),
            (Col::Hero, Share(13)),
            (Col::Eda, Px(76.0)),
            (Col::Rank, Px(64.0)),
            (Col::Length, Px(56.0)),
            (Col::Time, Px(48.0)),
            (Col::Crew, Px(72.0)),
            (Col::Unsure, Px(136.0)),
        ],
        Grid::Narrow => vec![
            (Col::Stripe, Px(4.0)),
            (Col::Result, Px(72.0)),
            (Col::Map, Share(10)),
            (Col::Rank, Px(64.0)),
            (Col::Time, Px(48.0)),
            (Col::Unsure, Px(120.0)),
        ],
    }
}

/// Sessions with headers, or one flat table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Grouping {
    #[default]
    Sessions,
    Flat,
}

/// The sort in effect: a column and its direction. `TIME ▼` by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub col: Col,
    pub desc: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self {
            col: Col::Time,
            desc: true,
        }
    }
}

impl Sort {
    /// A click on `col`'s header: the same column flips, another starts in
    /// its own first direction.
    pub fn clicked(self, col: Col) -> Self {
        if self.col == col {
            Self {
                col,
                desc: !self.desc,
            }
        } else {
            Self {
                col,
                desc: col.first_desc(),
            }
        }
    }

    /// The column sort in effect at `grid`: a sort on a column the grid
    /// hides (Hero or E/D/A under 1100 px) falls back to the default, so
    /// the rows never follow a column the header no longer shows.
    pub fn visible_in(self, grid: Grid) -> Self {
        if columns(grid).iter().any(|(c, _)| *c == self.col) {
            self
        } else {
            Sort::default()
        }
    }

    /// Sessions list oldest first only when the rows run oldest first.
    pub fn oldest_first(self) -> bool {
        self.col == Col::Time && !self.desc
    }
}

/// View state of the Games tab that is not a filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TabState {
    pub grouping: Grouping,
    pub sort: Sort,
    pub compact: bool,
    pub filters_open: bool,
}

impl TabState {
    pub fn row_height(&self) -> f32 {
        if self.compact { ROW_H_COMPACT } else { ROW_H }
    }
}

/// Where the Games tab's keyboard focus is. Iced 0.14 buttons take no
/// focus, so the tab keeps it here and draws the ring itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    None,
    /// A row, after the game above or below it was deleted.
    Row(String),
    /// The open row's Delete game button (back here after Cancel).
    DeleteButton,
    /// The confirm strip's Cancel (first) and Delete game.
    Cancel,
    ConfirmDelete,
}

/// The Games tab's state: the view settings, the focus, and whether the
/// "Game deleted." line is up.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GamesTab {
    pub view: TabState,
    pub focus: Focus,
    pub deleted_note: bool,
    /// The grid last drawn (the view sets it), so a delete from the keys
    /// finds the next row in the order the player sees.
    pub grid: std::cell::Cell<Grid>,
}

/// The row focus goes to after `sid` is deleted: the next row in `order`
/// (the rows as drawn), or the previous one when `sid` was the last.
pub fn neighbour(order: &[String], sid: &str) -> Option<String> {
    let at = order.iter().position(|s| s == sid)?;
    order
        .get(at + 1)
        .or_else(|| at.checked_sub(1).and_then(|i| order.get(i)))
        .cloned()
}

/// A row's sort value. `None` is unsure or empty, and always sorts last.
#[derive(Debug, Clone, PartialEq, PartialOrd)]
enum Key {
    Num(i64),
    Text(String),
}

fn key(g: &Game, col: Col) -> Option<Key> {
    match col {
        Col::Result => {
            if g.unsure_now("result") {
                return None;
            }
            match g.outcome {
                Outcome::Win => Some(Key::Num(2)),
                Outcome::Draw => Some(Key::Num(1)),
                Outcome::Loss => Some(Key::Num(0)),
                Outcome::Unknown => None,
            }
        }
        Col::Map => {
            let map = g.map_name.trim();
            (!map.is_empty() && !g.unsure_now("map")).then(|| Key::Text(map.to_lowercase()))
        }
        Col::Hero => {
            let hero = g.hero.trim();
            (!hero.is_empty() && !hero.eq_ignore_ascii_case("unknown") && !g.unsure_now("hero"))
                .then(|| Key::Text(hero.to_lowercase()))
        }
        // The column sorts by eliminations, so only an unsure E sends a
        // row last; an unsure D or A still reads amber in the cell.
        Col::Eda => (!g.unsure_now("e")).then_some(Key::Num(i64::from(g.elims))),
        Col::Time => Some(Key::Num(g.started_at().timestamp())),
        Col::Stripe | Col::Rank | Col::Length | Col::Crew | Col::Unsure => None,
    }
}

/// Sort `games` in place. Unsure and empty values go last in both
/// directions; ties keep newest first. Each row's key is built once.
pub fn sort_games(games: &mut Vec<&Game>, sort: Sort) {
    let mut keyed: Vec<(Option<Key>, DateTime<Utc>, &Game)> = games
        .iter()
        .map(|g| (key(g, sort.col), g.started_at(), *g))
        .collect();
    keyed.sort_by(|(ka, ta, _), (kb, tb, _)| {
        let order = match (ka, kb) {
            (Some(x), Some(y)) => {
                let o = x.partial_cmp(y).unwrap_or(Ordering::Equal);
                if sort.desc { o.reverse() } else { o }
            }
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        order.then_with(|| tb.cmp(ta))
    });
    *games = keyed.into_iter().map(|(_, _, g)| g).collect();
}

/// Elims, deaths or assists still unsure: E/D/A reads amber.
pub fn eda_unsure(g: &Game) -> bool {
    ["e", "d", "a"].iter().any(|f| g.unsure_now(f))
}

/// A read other than the six stats that is still unsure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsureItem {
    /// The map or its mode: one item, since an unread map stores both.
    Map,
    Hero,
    Result,
}

impl UnsureItem {
    pub fn name(self) -> &'static str {
        match self {
            UnsureItem::Map => "Map",
            UnsureItem::Hero => "Hero",
            UnsureItem::Result => "Result",
        }
    }
}

/// Map, hero and result reads still unsure that no other mark covers:
/// a missing map or hero already reads "Needs a look", and an undecided
/// (or still playing) game has no result to doubt.
pub fn unsure_items(g: &Game, in_progress: bool) -> Vec<UnsureItem> {
    let missing = g.review_fields();
    let mut out = Vec::new();
    if (g.unsure_now("map") || g.unsure_now("mode"))
        && !missing.contains(&"map")
        && !missing.contains(&"mode")
    {
        out.push(UnsureItem::Map);
    }
    if g.unsure_now("hero") && !missing.contains(&"hero") {
        out.push(UnsureItem::Hero);
    }
    if g.unsure_now("result") && g.outcome != Outcome::Unknown && !in_progress {
        out.push(UnsureItem::Result);
    }
    out
}

/// "Map unsure", "Map and hero unsure", "Map, hero and result unsure".
pub fn items_label(items: &[UnsureItem]) -> Option<String> {
    let names: Vec<String> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            if i == 0 {
                item.name().to_string()
            } else {
                item.name().to_lowercase()
            }
        })
        .collect();
    let list = match names.as_slice() {
        [] => return None,
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    Some(format!("{list} unsure"))
}

/// Result column text.
pub fn result_label(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Win => "WIN",
        Outcome::Loss => "LOSS",
        Outcome::Draw => "DRAW",
        Outcome::Unknown => "Undecided",
    }
}

/// `elims/deaths/assists`.
pub fn eda(game: &Game) -> String {
    format!("{}/{}/{}", game.elims, game.deaths, game.assists)
}

/// A mark at the end of a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    /// No outcome read: the chip opens the game to set one.
    NoOutcome,
    /// A map or hero is missing.
    NeedsLook,
    /// "2 stats unsure", or "Map unsure" for a read that is not a stat.
    Unsure(String),
    /// The site refused the upload.
    UploadRejected,
}

impl Mark {
    pub fn label(&self) -> String {
        match self {
            Mark::NoOutcome => "No outcome · Set".into(),
            Mark::NeedsLook => "Needs a look".into(),
            Mark::Unsure(label) => label.clone(),
            Mark::UploadRejected => crate::model::UPLOAD_REJECTED_LABEL.into(),
        }
    }

    /// Amber with the warning sign; the upload mark is red.
    pub fn is_warning(&self) -> bool {
        !matches!(self, Mark::UploadRejected)
    }
}

/// The row's one mark (the spec's row has one marker column), the most
/// urgent first: no outcome, a missing map or hero, an upload the website
/// refused, then unsure stats, then other unsure reads. The open panel
/// lists everything. A game in progress has no outcome yet, so no outcome
/// mark.
pub fn mark(g: &Game, in_progress: bool) -> Option<Mark> {
    if g.outcome == Outcome::Unknown && !in_progress {
        return Some(Mark::NoOutcome);
    }
    if g.needs_review() {
        return Some(Mark::NeedsLook);
    }
    if g.upload_reject.is_some() {
        return Some(Mark::UploadRejected);
    }
    g.unsure_label()
        .or_else(|| items_label(&unsure_items(g, in_progress)))
        .map(Mark::Unsure)
}

/// Result column text; a game still being played reads "Playing".
pub fn result_text(outcome: Outcome, in_progress: bool) -> &'static str {
    if in_progress {
        "Playing"
    } else {
        result_label(outcome)
    }
}

/// The map's mode from the stat-tracker catalog (Suravasa is Flashpoint).
pub fn mode_chip(map_name: &str) -> Option<&'static str> {
    stat_tracker::parse::map_mode(map_name)
}

/// One form-strip tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick {
    /// 10 px above the midline.
    Up,
    /// 10 px below the midline.
    Down,
    /// 4 px on the midline (draw or undecided).
    Mid,
}

/// Form strip of a session, oldest game first.
pub fn form_ticks(games: &[&Game]) -> Vec<Tick> {
    let mut ticks: Vec<(DateTime<Utc>, Tick)> = games
        .iter()
        .map(|g| {
            let t = match g.outcome {
                Outcome::Win => Tick::Up,
                Outcome::Loss => Tick::Down,
                Outcome::Draw | Outcome::Unknown => Tick::Mid,
            };
            (g.started_at(), t)
        })
        .collect();
    ticks.sort_by_key(|(at, _)| *at);
    ticks.into_iter().map(|(_, t)| t).collect()
}

/// "1 h 21 min", "48 min", "0 min".
pub fn duration_label(minutes: i64) -> String {
    let minutes = minutes.max(0);
    let (h, m) = (minutes / 60, minutes % 60);
    if h == 0 {
        format!("{m} min")
    } else {
        format!("{h} h {m} min")
    }
}

/// The day and part of it a local start time belongs to: "Sunday night"
/// with Sunday's date. Night runs 21:00 to 04:59 and belongs to the day it
/// started on, or to the day before for a start after midnight.
pub fn day_part<Tz: TimeZone>(start: DateTime<Tz>) -> (NaiveDate, String) {
    let today = start.date_naive();
    let (day, part) = match start.hour() {
        5..=11 => (today, "morning"),
        12..=16 => (today, "afternoon"),
        17..=20 => (today, "evening"),
        21..=23 => (today, "night"),
        _ => (today.pred_opt().unwrap_or(today), "night"),
    };
    (day, format!("{} {part}", day.format("%A")))
}

/// Session title and summary, in `tz`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHead {
    /// "Sunday night · Oct 18, 23:31 to 00:52".
    pub title: String,
    /// "5 games · 4-1 · 1 h 21 min".
    pub summary: String,
}

/// The header for one sitting in `tz`. It spans from the first game's
/// start to the last game's end (first and last Tab read; a row with no
/// reads kept counts as one moment). The date is the day the title names,
/// so a sitting that starts after midnight reads "Sunday night · Oct 18".
/// The record is wins-losses, with draws as a third number when there are
/// any, and undecided games after it ("1 undecided"), like Seasons. The
/// game still being played is not undecided yet. `shown` is how many of
/// the games the filters leave, said when it is fewer.
pub fn session_head_in<Tz>(
    games: &[&Game],
    shown: usize,
    live: Option<&str>,
    tz: &Tz,
) -> SessionHead
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    let first = games.iter().map(|g| g.started_at()).min();
    let last = games.iter().map(|g| g.ended_at()).max();
    let (Some(first), Some(last)) = (first, last) else {
        return SessionHead {
            title: String::new(),
            summary: "0 games".into(),
        };
    };
    let a = first.with_timezone(tz);
    let b = last.with_timezone(tz);
    let (day, part) = day_part(a.clone());
    let title = format!(
        "{part} · {}, {} to {}",
        day.format("%b %-d"),
        a.format("%H:%M"),
        b.format("%H:%M"),
    );
    let count = |o: Outcome| games.iter().filter(|g| g.outcome == o).count();
    let (wins, losses, draws) = (
        count(Outcome::Win),
        count(Outcome::Loss),
        count(Outcome::Draw),
    );
    let undecided = games
        .iter()
        .filter(|g| g.outcome == Outcome::Unknown)
        .filter(|g| !game_card_in_progress(&g.session_id, g.outcome, live))
        .count();
    let mut parts = vec![match games.len() {
        1 => "1 game".to_string(),
        n => format!("{n} games"),
    }];
    parts.push(if draws > 0 {
        format!("{wins}-{losses}-{draws}")
    } else {
        format!("{wins}-{losses}")
    });
    if undecided > 0 {
        parts.push(format!("{undecided} undecided"));
    }
    parts.push(duration_label((last - first).num_minutes()));
    if shown < games.len() {
        parts.push(format!("{shown} shown"));
    }
    SessionHead {
        title,
        summary: parts.join(" · "),
    }
}

/// Sittings, newest first: a new one starts when more than `gap` passes
/// between one game's end and the next game's start. Inside each, newest
/// first.
pub fn sittings(games: &[Game], gap: chrono::TimeDelta) -> Vec<Vec<&Game>> {
    let mut newest: Vec<&Game> = games.iter().collect();
    newest.sort_by_key(|g| std::cmp::Reverse(g.started_at()));
    let mut out: Vec<Vec<&Game>> = Vec::new();
    for g in newest {
        match out.last_mut() {
            Some(cur)
                if cur
                    .last()
                    .is_some_and(|newer| newer.started_at() - g.ended_at() <= gap) =>
            {
                cur.push(g);
            }
            _ => out.push(vec![g]),
        }
    }
    out
}

/// One block of the list: a session header (none in the flat list) and
/// its rows in sort order.
#[derive(Debug, Clone, PartialEq)]
pub struct Section<'a> {
    pub head: Option<SessionHead>,
    pub ticks: Vec<Tick>,
    pub rows: Vec<&'a Game>,
}

/// What the list draws.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout<'a> {
    pub sections: Vec<Section<'a>>,
    /// Rows left out past [`ROW_CAP`].
    pub hidden: usize,
}

/// The list's sections from every stored game (`all`) and the ones the
/// filters leave (`shown`). Sittings come from every game, so a filter
/// never splits one or changes its header; a sitting with no game shown is
/// left out. The sort orders rows inside a session, and the sessions run
/// oldest first only when the rows do. At most [`ROW_CAP`] rows.
pub fn layout_in<'a, Tz>(
    all: &'a [Game],
    shown: &[&'a Game],
    state: TabState,
    grid: Grid,
    gap_secs: u64,
    live: Option<&str>,
    tz: &Tz,
) -> Layout<'a>
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    let sort = state.sort.visible_in(grid);
    let mut left = ROW_CAP;
    let mut hidden = 0;
    let mut take = |mut rows: Vec<&'a Game>| {
        let keep = rows.len().min(left);
        hidden += rows.len() - keep;
        rows.truncate(keep);
        left -= keep;
        rows
    };
    let mut sections = Vec::new();
    match state.grouping {
        Grouping::Flat => {
            let mut rows = shown.to_vec();
            sort_games(&mut rows, sort);
            sections.push(Section {
                head: None,
                ticks: Vec::new(),
                rows: take(rows),
            });
        }
        Grouping::Sessions => {
            let ids: HashSet<&str> = shown.iter().map(|g| g.session_id.as_str()).collect();
            // A window of 0 is off: each game is its own session.
            let gap = if gap_secs == 0 {
                chrono::TimeDelta::MIN
            } else {
                crate::aggregate::secs_delta(gap_secs)
            };
            let mut groups = sittings(all, gap);
            if sort.oldest_first() {
                groups.reverse();
            }
            for group in groups {
                let mut rows: Vec<&Game> = group
                    .iter()
                    .copied()
                    .filter(|g| ids.contains(g.session_id.as_str()))
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                let head = session_head_in(&group, rows.len(), live, tz);
                sort_games(&mut rows, sort);
                let rows = take(rows);
                if rows.is_empty() {
                    continue;
                }
                sections.push(Section {
                    head: Some(head),
                    ticks: form_ticks(&group),
                    rows,
                });
            }
        }
    }
    Layout { sections, hidden }
}

/// [`layout_in`] in local time.
pub fn layout<'a>(
    all: &'a [Game],
    shown: &[&'a Game],
    state: TabState,
    grid: Grid,
    gap_secs: u64,
    live: Option<&str>,
) -> Layout<'a> {
    layout_in(all, shown, state, grid, gap_secs, live, &Local)
}

/// "Showing the first 500 of 812 games. Use the filters to narrow it."
pub fn cap_note(hidden: usize) -> Option<String> {
    (hidden > 0).then(|| {
        format!(
            "Showing the first {ROW_CAP} of {} games. Use the filters to narrow it.",
            ROW_CAP + hidden
        )
    })
}

/// The Time cell: "23:31", and in the flat list, where no session header
/// gives the day, "Oct 18" above it.
pub fn time_cell_in<Tz: TimeZone>(g: &Game, grouping: Grouping, tz: &Tz) -> (Option<String>, String)
where
    Tz::Offset: std::fmt::Display,
{
    let at = g.started_at().with_timezone(tz);
    let date = (grouping == Grouping::Flat).then(|| at.format("%b %-d").to_string());
    (date, at.format("%H:%M").to_string())
}

/// Under the last table (spec p2), after the split rule.
const SORT_NOTE: &str = "Columns sort; unsure and empty values always sort last. Rank, queue, length and crew are not recorded yet.";
/// No games stored at all.
pub const EMPTY_NONE: &str = "No games yet. A game shows here once the tracker saves it.";
/// Games stored, none left by the filters.
pub const EMPTY_FILTERED: &str = "No games match these filters.";

/// "Sessions split after 3 hours without a game." from the Session window
/// setting, which the grouping follows (ported from #555).
pub fn split_note(window_secs: u64) -> String {
    if window_secs == 0 {
        return format!("Session window is off, so each game is its own session. {SORT_NOTE}");
    }
    format!(
        "Sessions split after {} without a game. {SORT_NOTE}",
        crate::settings_model::SessionWindow(window_secs)
    )
}

/// "Filters" or "Filters (2)". The table draws the dropdown arrow after it
/// in iced's icon font.
pub fn filters_label(active: usize) -> String {
    if active == 0 {
        "Filters".into()
    } else {
        format!("Filters ({active})")
    }
}

/// The note under the list. The flat list has no sessions to split, so it
/// gets the sort note only.
pub fn list_note(grouping: Grouping, window_secs: u64) -> String {
    match grouping {
        Grouping::Sessions => split_note(window_secs),
        Grouping::Flat => SORT_NOTE.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Role, SegmentView};
    use chrono::{FixedOffset, TimeDelta};

    fn game(outcome: Outcome, at: &str, suspect: &[&str]) -> Game {
        let played_at = DateTime::parse_from_rfc3339(at)
            .unwrap()
            .with_timezone(&Utc);
        Game {
            session_id: at.to_string(),
            hero: "Wrecking Ball".into(),
            map_name: "Suravasa".into(),
            role: Role::Tank,
            outcome,
            elims: 28,
            deaths: 7,
            assists: 11,
            damage: 9000,
            healing: 0,
            mitigation: 0,
            played_at,
            edited: false,
            edited_fields: Vec::new(),
            ocr: crate::model::GameOcr::default(),
            segments: Vec::new(),
            upload_reject: None,
            suspect_fields: suspect.iter().map(|s| s.to_string()).collect(),
            synced: false,
            recognizer: String::new(),
        }
    }

    /// A game with one hero run from `start` to `end` (its Tab reads).
    fn timed(outcome: Outcome, start: &str, end: &str) -> Game {
        let mut g = game(outcome, end, &[]);
        let at = |t: &str| DateTime::parse_from_rfc3339(t).unwrap().with_timezone(&Utc);
        g.segments = vec![SegmentView {
            index: 0,
            hero: g.hero.clone(),
            role: g.role,
            snapshots: 4,
            confirmed: true,
            dismissed: false,
            resolution: None,
            first_seen: at(start),
            last_seen: at(end),
        }];
        g
    }

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    #[test]
    fn grid_switches_at_the_spec_window_widths() {
        assert_eq!(Grid::for_window(1440.0), Grid::Wide);
        // 1424 px: 24 px gutters leave 1376 px of content, the width a
        // 1440 window has. Still Mid, as the spec says under 1440.
        assert_eq!(Grid::for_window(1424.0), Grid::Mid);
        assert_eq!(Grid::for_window(1439.0), Grid::Mid);
        assert_eq!(Grid::for_window(1366.0), Grid::Mid);
        assert_eq!(Grid::for_window(1100.0), Grid::Mid);
        assert_eq!(Grid::for_window(1099.0), Grid::Narrow);
        assert_eq!(Grid::for_window(1024.0), Grid::Narrow);
    }

    #[test]
    fn narrow_hides_length_crew_eda_and_hero() {
        let cols: Vec<Col> = columns(Grid::Narrow).into_iter().map(|(c, _)| c).collect();
        for hidden in [Col::Length, Col::Crew, Col::Eda, Col::Hero] {
            assert!(!cols.contains(&hidden), "{hidden:?}");
        }
        assert_eq!(
            cols,
            [
                Col::Stripe,
                Col::Result,
                Col::Map,
                Col::Rank,
                Col::Time,
                Col::Unsure
            ]
        );
        assert_eq!(columns(Grid::Mid).len(), 10);
        assert_eq!(columns(Grid::Wide).len(), 10);
    }

    /// Spec r1.1: at 1024 the header hides Hero like the rows. The header
    /// labels come from the same column list, so HERO is not among them.
    #[test]
    fn narrow_header_hides_hero_like_the_rows() {
        let labels = |grid| -> Vec<&'static str> {
            columns(grid).into_iter().map(|(c, _)| c.label()).collect()
        };
        assert!(!labels(Grid::Narrow).contains(&"HERO"));
        assert!(!labels(Grid::Narrow).contains(&"E/D/A"));
        assert!(labels(Grid::Mid).contains(&"HERO"));
        assert!(labels(Grid::Wide).contains(&"HERO"));
    }

    #[test]
    fn a_row_has_one_mark_most_urgent_first() {
        let g = game(Outcome::Win, "2026-10-18T22:40:00Z", &["e", "d"]);
        assert_eq!(mark(&g, false), Some(Mark::Unsure("2 stats unsure".into())));
        let g = game(Outcome::Loss, "2026-10-18T22:40:00Z", &["e"]);
        assert_eq!(mark(&g, false).unwrap().label(), "1 stat unsure");
        let g = game(Outcome::Unknown, "2026-10-18T22:40:00Z", &["e"]);
        assert_eq!(mark(&g, false), Some(Mark::NoOutcome));
        assert_eq!(Mark::NoOutcome.label(), "No outcome · Set");
        // Still being played: no outcome yet is not a mark.
        assert_eq!(mark(&g, true), Some(Mark::Unsure("1 stat unsure".into())));
        assert_eq!(result_text(Outcome::Unknown, true), "Playing");
        let mut g = game(Outcome::Win, "2026-10-18T22:40:00Z", &[]);
        assert_eq!(mark(&g, false), None);
        g.map_name.clear();
        assert_eq!(mark(&g, false), Some(Mark::NeedsLook));
        g.map_name = "Suravasa".into();
        g.upload_reject = Some("bad".into());
        assert_eq!(mark(&g, false), Some(Mark::UploadRejected));
        assert!(!Mark::UploadRejected.is_warning());
        // A refused upload outranks unsure stats, so it is never hidden
        // behind an amber chip.
        g.suspect_fields = vec!["e".into()];
        assert_eq!(mark(&g, false), Some(Mark::UploadRejected));
        // No outcome and a missing map come first; the panel says the rest.
        g.outcome = Outcome::Unknown;
        assert_eq!(mark(&g, false), Some(Mark::NoOutcome));
        g.outcome = Outcome::Win;
        g.map_name.clear();
        assert_eq!(mark(&g, false), Some(Mark::NeedsLook));
    }

    /// Ported from #555; the store's flat names (`dmg`, `h`) against the
    /// edit names (`damage`, `healing`).
    #[test]
    fn unsure_count_skips_fixed_fields() {
        let mut g = game(Outcome::Win, "2026-10-18T22:40:00Z", &["dmg", "h"]);
        assert_eq!(g.unsure_label().as_deref(), Some("2 stats unsure"));
        g.edited_fields = vec!["damage".into()];
        assert_eq!(g.unsure_label().as_deref(), Some("1 stat unsure"));
        g.edited_fields.push("healing".into());
        assert_eq!(g.unsure_label(), None);
        assert!(!g.unsure_now("damage") && !g.unsure_now("dmg"));
    }

    /// An unread map stores both `map` and `mode`: one Map item, and no
    /// "stats unsure" count, since neither is a stat.
    #[test]
    fn an_unread_map_is_one_map_mark_not_two_stats() {
        let g = game(Outcome::Win, "2026-10-18T22:40:00Z", &["map", "mode"]);
        assert_eq!(g.unsure_label(), None);
        assert_eq!(unsure_items(&g, false), [UnsureItem::Map]);
        assert_eq!(mark(&g, false), Some(Mark::Unsure("Map unsure".into())));
        // Hero and result are their own items, never stats.
        let g = game(Outcome::Win, "2026-10-18T22:40:00Z", &["hero", "result"]);
        assert_eq!(g.unsure_label(), None);
        assert_eq!(
            mark(&g, false),
            Some(Mark::Unsure("Hero and result unsure".into()))
        );
        // An undecided game already says No outcome; no Result item.
        let g = game(Outcome::Unknown, "2026-10-18T22:40:00Z", &["result"]);
        assert_eq!(mark(&g, false), Some(Mark::NoOutcome));
        // A missing map already reads Needs a look.
        let mut g = game(Outcome::Win, "2026-10-18T22:40:00Z", &["map", "mode"]);
        g.map_name.clear();
        assert_eq!(mark(&g, false), Some(Mark::NeedsLook));
        // A hand edit of the map settles both names.
        let mut g = game(Outcome::Win, "2026-10-18T22:40:00Z", &["map", "mode"]);
        g.edited_fields = vec!["map_name".into()];
        assert!(unsure_items(&g, false).is_empty());
        assert_eq!(
            items_label(&[UnsureItem::Map, UnsureItem::Hero, UnsureItem::Result]).as_deref(),
            Some("Map, hero and result unsure")
        );
    }

    /// Unsure stats take the one mark over an unsure map; the panel lists
    /// the map too.
    #[test]
    fn unsure_stats_outrank_other_unsure_reads() {
        let g = game(Outcome::Win, "2026-10-18T22:40:00Z", &["dmg", "map"]);
        assert_eq!(mark(&g, false), Some(Mark::Unsure("1 stat unsure".into())));
        assert_eq!(unsure_items(&g, false), [UnsureItem::Map]);
    }

    /// `Game::from` keeps flat names only, so the app never sees `r3.dmg`.
    #[test]
    fn stored_row_names_reach_the_app_flat() {
        let stored = vec![
            "r3.dmg".to_string(),
            "dmg".to_string(),
            "x3.dmg".to_string(),
        ];
        assert_eq!(
            stat_tracker::reader_apply::upload_suspect_fields(&stored),
            ["dmg"]
        );
    }

    #[test]
    fn eda_reads_amber_while_e_d_or_a_is_unsure() {
        let mut g = game(Outcome::Win, "2026-10-18T22:40:00Z", &["a"]);
        assert!(eda_unsure(&g));
        g.edited_fields = vec!["assists".into()];
        assert!(!eda_unsure(&g));
        let g = game(Outcome::Win, "2026-10-18T22:40:00Z", &["dmg"]);
        assert!(!eda_unsure(&g));
    }

    #[test]
    fn result_labels_and_undecided() {
        assert_eq!(result_label(Outcome::Win), "WIN");
        assert_eq!(result_label(Outcome::Loss), "LOSS");
        assert_eq!(result_label(Outcome::Draw), "DRAW");
        assert_eq!(result_label(Outcome::Unknown), "Undecided");
    }

    #[test]
    fn suravasa_reads_flashpoint() {
        assert_eq!(mode_chip("Suravasa"), Some("Flashpoint"));
        // Spec r1.1: Circuit Royal's row shows the Escort chip.
        assert_eq!(mode_chip("Circuit Royal"), Some("Escort"));
        assert_eq!(mode_chip("King's Row"), Some("Hybrid"));
        assert_eq!(mode_chip(""), None);
        assert_eq!(mode_chip("Not A Map"), None);
    }

    #[test]
    fn duration_labels() {
        assert_eq!(duration_label(81), "1 h 21 min");
        assert_eq!(duration_label(48), "48 min");
        assert_eq!(duration_label(0), "0 min");
        assert_eq!(duration_label(-5), "0 min");
        assert_eq!(duration_label(120), "2 h 0 min");
    }

    #[test]
    fn day_part_after_midnight_is_the_night_before() {
        let tz = utc();
        let sun_2331 = tz.with_ymd_and_hms(2026, 10, 18, 23, 31, 0).unwrap();
        assert_eq!(day_part(sun_2331).1, "Sunday night");
        let mon_0040 = tz.with_ymd_and_hms(2026, 10, 19, 0, 40, 0).unwrap();
        let (day, part) = day_part(mon_0040);
        assert_eq!(part, "Sunday night");
        assert_eq!(day.format("%b %-d").to_string(), "Oct 18");
        let mon_1300 = tz.with_ymd_and_hms(2026, 10, 19, 13, 0, 0).unwrap();
        assert_eq!(day_part(mon_1300).1, "Monday afternoon");
    }

    #[test]
    fn session_head_matches_the_spec_sample() {
        // Spec S1: Sunday night, Oct 18, 23:31 to 00:52, 5 games, 4-1.
        let games = [
            game(Outcome::Win, "2026-10-19T00:52:00Z", &[]),
            game(Outcome::Win, "2026-10-19T00:25:00Z", &[]),
            game(Outcome::Loss, "2026-10-19T00:05:00Z", &[]),
            game(Outcome::Win, "2026-10-18T23:42:00Z", &[]),
            game(Outcome::Win, "2026-10-18T23:31:00Z", &[]),
        ];
        let groups = sittings(&games, TimeDelta::hours(3));
        assert_eq!(groups.len(), 1);
        let head = session_head_in(&groups[0], 5, None, &utc());
        assert_eq!(head.title, "Sunday night · Oct 18, 23:31 to 00:52");
        assert_eq!(head.summary, "5 games · 4-1 · 1 h 21 min");
        assert_eq!(
            form_ticks(&groups[0]),
            [Tick::Up, Tick::Up, Tick::Down, Tick::Up, Tick::Up]
        );
    }

    /// A sitting that starts after midnight is dated by the night it
    /// belongs to, so the title never reads "Sunday night · Oct 19".
    #[test]
    fn an_after_midnight_title_dates_the_night_it_names() {
        let games = [
            game(Outcome::Win, "2026-10-19T01:30:00Z", &[]),
            game(Outcome::Win, "2026-10-19T00:40:00Z", &[]),
        ];
        let groups = sittings(&games, TimeDelta::hours(3));
        let head = session_head_in(&groups[0], 2, None, &utc());
        assert_eq!(head.title, "Sunday night · Oct 18, 00:40 to 01:30");
    }

    /// Undecided games are their own tail, like Seasons; draws are the
    /// third number; the game still being played is not undecided.
    #[test]
    fn undecided_games_follow_the_record() {
        let games = [
            game(Outcome::Unknown, "2026-10-18T00:30:00Z", &[]),
            game(Outcome::Win, "2026-10-18T00:14:00Z", &[]),
            game(Outcome::Unknown, "2026-10-17T23:11:00Z", &[]),
            game(Outcome::Loss, "2026-10-17T22:33:00Z", &[]),
        ];
        let groups = sittings(&games, TimeDelta::hours(3));
        let live = Some("2026-10-18T00:30:00Z");
        let head = session_head_in(&groups[0], 4, live, &utc());
        assert_eq!(head.summary, "4 games · 1-1 · 1 undecided · 1 h 57 min");
        let head = session_head_in(&groups[0], 4, None, &utc());
        assert_eq!(head.summary, "4 games · 1-1 · 2 undecided · 1 h 57 min");
        let draws = [
            game(Outcome::Draw, "2026-10-18T00:14:00Z", &[]),
            game(Outcome::Win, "2026-10-17T23:50:00Z", &[]),
        ];
        let groups = sittings(&draws, TimeDelta::hours(3));
        let head = session_head_in(&groups[0], 1, None, &utc());
        assert_eq!(head.summary, "2 games · 1-0-1 · 24 min · 1 shown");
        assert_eq!(form_ticks(&groups[0]), [Tick::Up, Tick::Mid]);
    }

    /// The gap runs from one game's end to the next game's start, so a
    /// 20-minute game and a 10-minute queue stay one sitting under a 30
    /// minute window, and the span covers the last game's length.
    #[test]
    fn sittings_split_on_the_gap_between_games_not_their_starts() {
        let games = [
            timed(Outcome::Win, "2026-10-18T20:30:00Z", "2026-10-18T20:50:00Z"),
            timed(
                Outcome::Loss,
                "2026-10-18T20:00:00Z",
                "2026-10-18T20:20:00Z",
            ),
            timed(Outcome::Win, "2026-10-18T18:00:00Z", "2026-10-18T18:15:00Z"),
        ];
        let groups = sittings(&games, TimeDelta::minutes(30));
        assert_eq!(groups.iter().map(Vec::len).collect::<Vec<_>>(), [2, 1]);
        let head = session_head_in(&groups[0], 2, None, &utc());
        assert_eq!(head.title, "Sunday evening · Oct 18, 20:00 to 20:50");
        assert_eq!(head.summary, "2 games · 1-1 · 50 min");
    }

    fn ids(games: &[&Game]) -> Vec<String> {
        games.iter().map(|g| g.session_id.clone()).collect()
    }

    /// A filter hides rows; it never splits a sitting or rewrites its
    /// header, and a sitting with nothing shown is left out.
    #[test]
    fn filters_keep_sittings_whole() {
        let all = vec![
            game(Outcome::Win, "2026-10-18T22:50:00Z", &[]),
            game(Outcome::Loss, "2026-10-18T22:20:00Z", &[]),
            game(Outcome::Win, "2026-10-18T21:50:00Z", &[]),
            game(Outcome::Win, "2026-10-17T21:50:00Z", &[]),
        ];
        // A filter that drops the middle game of the first sitting.
        let shown: Vec<&Game> = vec![&all[0], &all[2]];
        let l = layout_in(
            &all,
            &shown,
            TabState::default(),
            Grid::Wide,
            1800,
            None,
            &utc(),
        );
        assert_eq!(l.sections.len(), 1);
        let s = &l.sections[0];
        assert_eq!(ids(&s.rows), ids(&[&all[0], &all[2]]));
        let head = s.head.as_ref().unwrap();
        assert_eq!(head.summary, "3 games · 2-1 · 1 h 0 min · 2 shown");
        assert_eq!(s.ticks.len(), 3);
        assert_eq!(l.hidden, 0);
    }

    #[test]
    fn the_flat_list_is_one_section_with_no_header() {
        let all = vec![
            game(Outcome::Win, "2026-10-18T22:50:00Z", &[]),
            game(Outcome::Loss, "2026-10-17T22:20:00Z", &[]),
        ];
        let shown: Vec<&Game> = all.iter().collect();
        let state = TabState {
            grouping: Grouping::Flat,
            ..TabState::default()
        };
        let l = layout_in(&all, &shown, state, Grid::Wide, 1800, None, &utc());
        assert_eq!(l.sections.len(), 1);
        assert!(l.sections[0].head.is_none());
        assert_eq!(l.sections[0].rows.len(), 2);
        // Flat rows carry their date; session rows have the header's.
        let (date, hm) = time_cell_in(&all[1], Grouping::Flat, &utc());
        assert_eq!((date.as_deref(), hm.as_str()), (Some("Oct 17"), "22:20"));
        assert_eq!(time_cell_in(&all[1], Grouping::Sessions, &utc()).0, None);
        assert!(!list_note(Grouping::Flat, 1800).contains("Sessions split"));
        assert!(list_note(Grouping::Sessions, 1800).contains("Sessions split"));
    }

    /// Oldest first lists the sessions oldest first too.
    #[test]
    fn oldest_first_reverses_the_sessions() {
        let all = vec![
            game(Outcome::Win, "2026-10-18T22:50:00Z", &[]),
            game(Outcome::Loss, "2026-10-17T22:20:00Z", &[]),
        ];
        let shown: Vec<&Game> = all.iter().collect();
        let state = TabState {
            sort: Sort::default().clicked(Col::Time),
            ..TabState::default()
        };
        let l = layout_in(&all, &shown, state, Grid::Wide, 1800, None, &utc());
        assert_eq!(l.sections[0].rows[0].session_id, all[1].session_id);
    }

    #[test]
    fn the_list_stops_at_the_row_cap() {
        let start = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let all: Vec<Game> = (0..ROW_CAP + 7)
            .map(|i| {
                let at = (start + TimeDelta::hours(i as i64 * 5)).to_rfc3339();
                game(Outcome::Win, &at, &[])
            })
            .collect();
        let shown: Vec<&Game> = all.iter().collect();
        for grouping in [Grouping::Sessions, Grouping::Flat] {
            let state = TabState {
                grouping,
                ..TabState::default()
            };
            let l = layout_in(&all, &shown, state, Grid::Wide, 1800, None, &utc());
            let rows: usize = l.sections.iter().map(|s| s.rows.len()).sum();
            assert_eq!((rows, l.hidden), (ROW_CAP, 7), "{grouping:?}");
        }
        assert_eq!(
            cap_note(7).as_deref(),
            Some("Showing the first 500 of 507 games. Use the filters to narrow it.")
        );
        assert_eq!(cap_note(0), None);
    }

    /// Under 1100 px Hero and E/D/A are hidden, so a sort on them falls
    /// back to the default instead of ordering by an unseen column.
    #[test]
    fn a_sort_on_a_hidden_column_falls_back() {
        let eda = Sort::default().clicked(Col::Eda);
        assert_eq!(eda.visible_in(Grid::Wide), eda);
        assert_eq!(eda.visible_in(Grid::Narrow), Sort::default());
        let hero = Sort::default().clicked(Col::Hero);
        assert_eq!(hero.visible_in(Grid::Narrow), Sort::default());
        let map = Sort::default().clicked(Col::Map);
        assert_eq!(map.visible_in(Grid::Narrow), map);
    }

    #[test]
    fn time_sorts_newest_first_by_default() {
        let a = game(Outcome::Win, "2026-10-18T22:01:00Z", &[]);
        let b = game(Outcome::Loss, "2026-10-18T22:03:00Z", &[]);
        let c = game(Outcome::Win, "2026-10-18T22:02:00Z", &[]);
        let mut rows = vec![&a, &b, &c];
        sort_games(&mut rows, Sort::default());
        assert_eq!(ids(&rows), [&b, &c, &a].map(|g| g.session_id.clone()));
        sort_games(&mut rows, Sort::default().clicked(Col::Time));
        assert_eq!(ids(&rows), [&a, &c, &b].map(|g| g.session_id.clone()));
        assert!(Sort::default().desc);
        assert!(Sort::default().clicked(Col::Time).oldest_first());
    }

    /// Ported from #555.
    #[test]
    fn unsure_and_empty_values_sort_last_both_ways() {
        let with = |at: &str, outcome, elims, suspect: &[&str]| {
            let mut g = game(outcome, at, suspect);
            g.elims = elims;
            g
        };
        let a = with("2026-10-18T22:01:00Z", Outcome::Win, 30, &[]);
        let mut b = with("2026-10-18T22:02:00Z", Outcome::Win, 40, &["e"]);
        let c = with("2026-10-18T22:03:00Z", Outcome::Win, 10, &[]);
        let undecided = with("2026-10-18T22:04:00Z", Outcome::Unknown, 5, &[]);
        for desc in [true, false] {
            let mut rows = vec![&a, &b, &c];
            sort_games(
                &mut rows,
                Sort {
                    col: Col::Eda,
                    desc,
                },
            );
            assert_eq!(rows.last().unwrap().session_id, b.session_id, "desc {desc}");
            let mut rows = vec![&undecided, &a, &b];
            sort_games(
                &mut rows,
                Sort {
                    col: Col::Result,
                    desc,
                },
            );
            assert_eq!(
                rows.last().unwrap().session_id,
                undecided.session_id,
                "desc {desc}"
            );
        }
        // A hand edit settles the unsure value.
        b.edited_fields = vec!["elims".into()];
        let mut rows = vec![&a, &b, &c];
        sort_games(
            &mut rows,
            Sort {
                col: Col::Eda,
                desc: true,
            },
        );
        assert_eq!(ids(&rows), [&b, &a, &c].map(|g| g.session_id.clone()));
    }

    /// E/D/A sorts by eliminations; an unsure death count does not send a
    /// row with a sure E last.
    #[test]
    fn eda_sorts_last_only_on_an_unsure_e() {
        let mut a = game(Outcome::Win, "2026-10-18T22:01:00Z", &["d"]);
        a.elims = 40;
        let b = game(Outcome::Win, "2026-10-18T22:02:00Z", &[]);
        let mut rows = vec![&b, &a];
        sort_games(
            &mut rows,
            Sort {
                col: Col::Eda,
                desc: true,
            },
        );
        assert_eq!(ids(&rows), ids(&[&a, &b]));
    }

    #[test]
    fn a_header_click_flips_or_starts_in_the_columns_own_direction() {
        let s = Sort::default().clicked(Col::Time);
        assert_eq!(
            s,
            Sort {
                col: Col::Time,
                desc: false
            }
        );
        assert_eq!(
            s.clicked(Col::Map),
            Sort {
                col: Col::Map,
                desc: false
            }
        );
        assert_eq!(
            s.clicked(Col::Eda),
            Sort {
                col: Col::Eda,
                desc: true
            }
        );
        assert!(!Col::Rank.sortable() && !Col::Crew.sortable());
        assert!(Col::Hero.sortable());
    }

    #[test]
    fn compact_rows_are_shorter() {
        let mut s = TabState::default();
        assert_eq!(s.row_height(), 40.0);
        s.compact = true;
        assert_eq!(s.row_height(), 32.0);
        assert_eq!(s.grouping, Grouping::Sessions);
    }

    #[test]
    fn split_note_and_filters_label() {
        assert!(split_note(10_800).starts_with("Sessions split after 3 hours without a game."));
        assert!(split_note(1800).contains("30 min"));
        assert!(split_note(3600).starts_with("Sessions split after 1 hour "));
        assert!(split_note(0).contains("each game is its own session"));
        assert!(split_note(0).contains("unsure and empty values always sort last"));
        assert_eq!(filters_label(0), "Filters");
        assert_eq!(filters_label(1), "Filters (1)");
    }

    #[test]
    fn empty_states() {
        assert!(EMPTY_NONE.starts_with("No games yet."));
        assert_eq!(EMPTY_FILTERED, "No games match these filters.");
    }

    #[test]
    fn focus_after_a_delete_goes_to_the_next_row_else_the_previous() {
        let order: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        assert_eq!(neighbour(&order, "a").as_deref(), Some("b"));
        assert_eq!(neighbour(&order, "b").as_deref(), Some("c"));
        assert_eq!(neighbour(&order, "c").as_deref(), Some("b"));
        assert_eq!(neighbour(&order[..1], "a"), None);
        assert_eq!(neighbour(&order, "gone"), None);
    }
}
