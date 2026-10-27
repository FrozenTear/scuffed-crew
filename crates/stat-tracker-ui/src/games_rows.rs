//! Pure layout and labels for the 0.6 Games table (variant A, top tabs):
//! which columns fit, session headers and their summary line, the form
//! strip, result labels and row marks, and the sort (ported from #555).
//! No iced types here, so every rule has a unit test;
//! [`crate::games_table`] draws it.

use std::cmp::Ordering;

use chrono::{DateTime, Local, TimeZone, Timelike, Utc};

use crate::aggregate::SessionGroup;
use crate::model::{Game, Outcome};
use stat_tracker::suspect;

/// Content width (the page minus its side padding) from which the wide
/// grid applies. The spec switches at a 1440 px window; the page pads
/// 32 px a side there.
pub const WIDE_FROM: f32 = 1376.0;
/// Content width under which Length, Crew, E/D/A and Hero are hidden. The
/// spec switches at a 1100 px window; the page pads 24 px a side there.
pub const NARROW_UNDER: f32 = 1052.0;

/// Row height in px.
pub const ROW_H: f32 = 40.0;
/// Row height with Compact rows on.
pub const ROW_H_COMPACT: f32 = 32.0;
/// Column header height in px.
pub const HEADER_H: f32 = 32.0;

/// Which grid the table uses at a content width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grid {
    /// 1440 px windows and up.
    Wide,
    /// 1100 to 1439 px windows.
    Mid,
    /// Under 1100 px: Length, Crew, E/D/A and Hero are hidden.
    Narrow,
}

impl Grid {
    pub fn for_width(content_width: f32) -> Self {
        if content_width >= WIDE_FROM {
            Grid::Wide
        } else if content_width >= NARROW_UNDER {
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

    pub fn arrow(self) -> &'static str {
        if self.desc { "▼" } else { "▲" }
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

/// A row's sort value. `None` is unsure or empty, and always sorts last.
#[derive(Debug, Clone, PartialEq, PartialOrd)]
enum Key {
    Num(i64),
    Text(String),
}

fn key(g: &Game, col: Col) -> Option<Key> {
    match col {
        Col::Result => {
            if unsure_now(g, "result") {
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
            (!map.is_empty() && !unsure_now(g, "map")).then(|| Key::Text(map.to_lowercase()))
        }
        Col::Hero => {
            let hero = g.hero.trim();
            (!hero.is_empty() && !hero.eq_ignore_ascii_case("unknown") && !unsure_now(g, "hero"))
                .then(|| Key::Text(hero.to_lowercase()))
        }
        Col::Eda => (!eda_unsure(g)).then_some(Key::Num(i64::from(g.elims))),
        Col::Time => Some(Key::Num(g.played_at.timestamp())),
        Col::Stripe | Col::Rank | Col::Length | Col::Crew | Col::Unsure => None,
    }
}

/// Sort `games` in place. Unsure and empty values go last in both
/// directions; ties keep newest first.
pub fn sort_games(games: &mut [&Game], sort: Sort) {
    games.sort_by(|a, b| {
        let order = match (key(a, sort.col), key(b, sort.col)) {
            (Some(x), Some(y)) => {
                let o = x.partial_cmp(&y).unwrap_or(Ordering::Equal);
                if sort.desc { o.reverse() } else { o }
            }
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        order.then_with(|| b.played_at.cmp(&a.played_at))
    });
}

/// The flat suspect names `field` stands for: an edit name (`damage`)
/// maps through stat-tracker-core's table, a flat name (`dmg`) is itself.
fn flats_for(field: &str) -> Vec<&str> {
    match suspect::suspect_names_for_edit(field) {
        [] => suspect::flat_suspect_name(field).into_iter().collect(),
        names => names.to_vec(),
    }
}

/// A flat name the reader stored (as `dmg` or a row name like `r3.dmg`)
/// that no hand edit has fixed since. Both sides use
/// `stat_tracker::suspect`, the rules the store and uploader apply.
fn flat_unsure_now(g: &Game, flat: &str) -> bool {
    g.suspect_fields
        .iter()
        .any(|s| suspect::flat_suspect_name(s) == Some(flat))
        && !g
            .edited_fields
            .iter()
            .any(|e| suspect::suspect_names_for_edit(e).contains(&flat))
}

/// A field the reader was unsure of that no hand edit has fixed since.
/// `field` is a flat name (`e`, `dmg`) or an edit name (`elims`, `damage`).
pub fn unsure_now(g: &Game, field: &str) -> bool {
    flats_for(field).into_iter().any(|f| flat_unsure_now(g, f))
}

/// Elims, deaths or assists still unsure: E/D/A reads amber.
pub fn eda_unsure(g: &Game) -> bool {
    ["e", "d", "a"].iter().any(|f| unsure_now(g, f))
}

/// "2 stats unsure": the fields still unsure. `None` when there are none.
pub fn unsure_label(g: &Game) -> Option<String> {
    // Distinct flat names, so `dmg` and a leftover `r3.dmg` count once.
    let mut flats: Vec<&str> = g
        .suspect_fields
        .iter()
        .filter_map(|s| suspect::flat_suspect_name(s))
        .filter(|f| flat_unsure_now(g, f))
        .collect();
    flats.sort_unstable();
    flats.dedup();
    let n = flats.len();
    match n {
        0 => None,
        1 => Some("1 stat unsure".to_string()),
        n => Some(format!("{n} stats unsure")),
    }
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
    /// "2 stats unsure".
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

/// The row's marks: what still needs a look, then an upload the website
/// refused. A game in progress has no outcome yet, so no outcome mark.
/// The rejected upload shows only when nothing else does: the spec's row
/// has one marker column (r1.1 draws the rejected undecided game with
/// "No outcome · Set" alone), and the open panel always says why.
pub fn marks(g: &Game, in_progress: bool) -> Vec<Mark> {
    let mut out = Vec::new();
    if g.outcome == Outcome::Unknown && !in_progress {
        out.push(Mark::NoOutcome);
    } else if g.needs_review() {
        out.push(Mark::NeedsLook);
    }
    if let Some(label) = unsure_label(g) {
        out.push(Mark::Unsure(label));
    }
    if g.upload_reject.is_some() && out.is_empty() {
        out.push(Mark::UploadRejected);
    }
    out
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
pub fn form_ticks(games: &[Game]) -> Vec<Tick> {
    let mut ticks: Vec<(DateTime<Utc>, Tick)> = games
        .iter()
        .map(|g| {
            let t = match g.outcome {
                Outcome::Win => Tick::Up,
                Outcome::Loss => Tick::Down,
                Outcome::Draw | Outcome::Unknown => Tick::Mid,
            };
            (g.played_at, t)
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

/// "Sunday night", from the local start time. Night runs 21:00 to 04:59
/// and belongs to the day it started on, or to the day before for a start
/// after midnight.
pub fn day_part<Tz: TimeZone>(start: DateTime<Tz>) -> String {
    let hour = start.hour();
    let (day, part) = match hour {
        5..=11 => (start.date_naive(), "morning"),
        12..=16 => (start.date_naive(), "afternoon"),
        17..=20 => (start.date_naive(), "evening"),
        21..=23 => (start.date_naive(), "night"),
        _ => (
            start.date_naive().pred_opt().unwrap_or(start.date_naive()),
            "night",
        ),
    };
    format!("{} {part}", day.format("%A"))
}

/// Session title and summary, in `tz`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHead {
    /// "Sunday night · Oct 18, 23:31 to 00:52".
    pub title: String,
    /// "5 games · 4-1 · 1 h 21 min".
    pub summary: String,
}

/// The header for `group` in `tz`. The span runs from the first game to
/// the last (game start times; game length is not stored yet). Undecided
/// games count in the record as the third number, like draws.
pub fn session_head_in<Tz>(group: &SessionGroup, tz: &Tz) -> SessionHead
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    let first = group.games.iter().map(|g| g.played_at).min();
    let last = group.games.iter().map(|g| g.played_at).max();
    let (Some(first), Some(last)) = (first, last) else {
        return SessionHead {
            title: String::new(),
            summary: "0 games".into(),
        };
    };
    let a = first.with_timezone(tz);
    let b = last.with_timezone(tz);
    let title = format!(
        "{} · {}, {} to {}",
        day_part(a.clone()),
        a.format("%b %-d"),
        a.format("%H:%M"),
        b.format("%H:%M"),
    );
    let wins = group.record.wins;
    let losses = group.record.losses;
    let other = group.games.len().saturating_sub(wins + losses);
    let record = if other > 0 {
        format!("{wins}-{losses}-{other}")
    } else {
        format!("{wins}-{losses}")
    };
    let n = group.games.len();
    let games = if n == 1 {
        "1 game".to_string()
    } else {
        format!("{n} games")
    };
    SessionHead {
        title,
        summary: format!(
            "{games} · {record} · {}",
            duration_label((last - first).num_minutes())
        ),
    }
}

pub fn session_head(group: &SessionGroup) -> SessionHead {
    session_head_in(group, &Local)
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

/// "Filters" or "Filters (2)".
pub fn filters_label(active: usize) -> String {
    if active == 0 {
        "Filters ▾".into()
    } else {
        format!("Filters ({active}) ▾")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::group_by_session_gap;
    use crate::model::Role;
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

    #[test]
    fn grid_switches_at_the_spec_breakpoints() {
        // 1440 window with 32 px padding a side, 1366 and 1024 windows.
        assert_eq!(Grid::for_width(1440.0 - 64.0), Grid::Wide);
        assert_eq!(Grid::for_width(1366.0 - 64.0), Grid::Mid);
        assert_eq!(Grid::for_width(1100.0 - 48.0), Grid::Mid);
        assert_eq!(Grid::for_width(1024.0 - 48.0), Grid::Narrow);
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
    fn row_marks() {
        let g = game(Outcome::Win, "2026-10-18T22:40:00Z", &["e", "d"]);
        assert_eq!(marks(&g, false), [Mark::Unsure("2 stats unsure".into())]);
        let g = game(Outcome::Loss, "2026-10-18T22:40:00Z", &["e"]);
        assert_eq!(marks(&g, false)[0].label(), "1 stat unsure");
        let g = game(Outcome::Unknown, "2026-10-18T22:40:00Z", &["e"]);
        assert_eq!(
            marks(&g, false),
            [Mark::NoOutcome, Mark::Unsure("1 stat unsure".into())]
        );
        assert_eq!(Mark::NoOutcome.label(), "No outcome · Set");
        // Still being played: no outcome yet is not a mark.
        assert_eq!(marks(&g, true), [Mark::Unsure("1 stat unsure".into())]);
        assert_eq!(result_text(Outcome::Unknown, true), "Playing");
        let mut g = game(Outcome::Win, "2026-10-18T22:40:00Z", &[]);
        assert!(marks(&g, false).is_empty());
        g.map_name.clear();
        assert_eq!(marks(&g, false), [Mark::NeedsLook]);
        g.map_name = "Suravasa".into();
        g.upload_reject = Some("bad".into());
        assert_eq!(marks(&g, false), [Mark::UploadRejected]);
        assert!(!Mark::UploadRejected.is_warning());
        // Beside another mark it would not fit the column; the panel says it.
        g.outcome = Outcome::Unknown;
        assert_eq!(marks(&g, false), [Mark::NoOutcome]);
    }

    /// Ported from #555; the store's flat names (`dmg`, `h`) against the
    /// edit names (`damage`, `healing`).
    #[test]
    fn unsure_count_skips_fixed_fields() {
        let mut g = game(Outcome::Win, "2026-10-18T22:40:00Z", &["dmg", "h"]);
        assert_eq!(unsure_label(&g).as_deref(), Some("2 stats unsure"));
        g.edited_fields = vec!["damage".into()];
        assert_eq!(unsure_label(&g).as_deref(), Some("1 stat unsure"));
        g.edited_fields.push("healing".into());
        assert_eq!(unsure_label(&g), None);
    }

    /// Older rows store row-prefixed names; `r3.dmg` is damage unsure.
    #[test]
    fn a_row_prefixed_name_counts_as_its_stat() {
        let mut g = game(Outcome::Win, "2026-10-18T22:40:00Z", &["r3.dmg"]);
        assert!(unsure_now(&g, "dmg"));
        assert!(unsure_now(&g, "damage"));
        assert!(!unsure_now(&g, "h"));
        assert_eq!(unsure_label(&g).as_deref(), Some("1 stat unsure"));
        g.suspect_fields.push("dmg".into());
        assert_eq!(unsure_label(&g).as_deref(), Some("1 stat unsure"));
        assert!(crate::stat_tiles::unsure_now(&g, "damage"));
        g.edited_fields = vec!["damage".into()];
        assert!(!unsure_now(&g, "dmg"));
        assert_eq!(unsure_label(&g), None);
        // Not a row name: left alone, as the store does.
        let g = game(Outcome::Win, "2026-10-18T22:40:00Z", &["x3.dmg"]);
        assert!(!unsure_now(&g, "dmg"));
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
        let tz = FixedOffset::east_opt(0).unwrap();
        let sun_2331 = tz.with_ymd_and_hms(2026, 10, 18, 23, 31, 0).unwrap();
        assert_eq!(day_part(sun_2331), "Sunday night");
        let mon_0040 = tz.with_ymd_and_hms(2026, 10, 19, 0, 40, 0).unwrap();
        assert_eq!(day_part(mon_0040), "Sunday night");
        let mon_1300 = tz.with_ymd_and_hms(2026, 10, 19, 13, 0, 0).unwrap();
        assert_eq!(day_part(mon_1300), "Monday afternoon");
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
        let refs: Vec<&Game> = games.iter().collect();
        let groups = group_by_session_gap(&refs, TimeDelta::hours(3));
        assert_eq!(groups.len(), 1);
        let head = session_head_in(&groups[0], &FixedOffset::east_opt(0).unwrap());
        assert_eq!(head.title, "Sunday night · Oct 18, 23:31 to 00:52");
        assert_eq!(head.summary, "5 games · 4-1 · 1 h 21 min");
        assert_eq!(
            form_ticks(&groups[0].games),
            [Tick::Up, Tick::Up, Tick::Down, Tick::Up, Tick::Up]
        );
    }

    #[test]
    fn undecided_is_the_third_record_number() {
        let games = [
            game(Outcome::Win, "2026-10-18T00:14:00Z", &[]),
            game(Outcome::Unknown, "2026-10-17T23:11:00Z", &[]),
            game(Outcome::Loss, "2026-10-17T22:33:00Z", &[]),
        ];
        let refs: Vec<&Game> = games.iter().collect();
        let groups = group_by_session_gap(&refs, TimeDelta::hours(3));
        let head = session_head_in(&groups[0], &FixedOffset::east_opt(0).unwrap());
        assert_eq!(head.summary, "3 games · 1-1-1 · 1 h 41 min");
        assert_eq!(
            form_ticks(&groups[0].games),
            [Tick::Down, Tick::Mid, Tick::Up]
        );
    }

    fn ids(games: &[&Game]) -> Vec<String> {
        games.iter().map(|g| g.session_id.clone()).collect()
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
        assert_eq!(Sort::default().arrow(), "▼");
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
        assert_eq!(filters_label(0), "Filters ▾");
        assert_eq!(filters_label(1), "Filters (1) ▾");
    }

    #[test]
    fn empty_states() {
        assert!(EMPTY_NONE.starts_with("No games yet."));
        assert_eq!(EMPTY_FILTERED, "No games match these filters.");
    }
}
