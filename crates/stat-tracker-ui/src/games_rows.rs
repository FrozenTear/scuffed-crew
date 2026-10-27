//! Pure layout and labels for the 0.6 Games table (variant A, top tabs):
//! which columns fit, session headers and their summary line, the form
//! strip, result and unsure labels, and the row order. No iced types here,
//! so every rule has a unit test; [`crate::games_table`] draws it.

use chrono::{DateTime, Local, TimeZone, Timelike, Utc};

use crate::aggregate::SessionGroup;
use crate::model::{Game, Outcome};

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

/// Order of rows by time. Newest first is the default (`TIME ▼`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeSort {
    #[default]
    NewestFirst,
    OldestFirst,
}

impl TimeSort {
    pub fn flipped(self) -> Self {
        match self {
            TimeSort::NewestFirst => TimeSort::OldestFirst,
            TimeSort::OldestFirst => TimeSort::NewestFirst,
        }
    }

    pub fn arrow(self) -> &'static str {
        match self {
            TimeSort::NewestFirst => "▼",
            TimeSort::OldestFirst => "▲",
        }
    }
}

/// View state of the Games tab that is not a filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TabState {
    pub grouping: Grouping,
    pub sort: TimeSort,
    pub compact: bool,
    pub filters_open: bool,
}

impl TabState {
    pub fn row_height(&self) -> f32 {
        if self.compact { ROW_H_COMPACT } else { ROW_H }
    }
}

/// `games` (newest first, as the store lists them) in `sort` order.
pub fn sorted<'a>(games: &[&'a Game], sort: TimeSort) -> Vec<&'a Game> {
    let mut out = games.to_vec();
    match sort {
        TimeSort::NewestFirst => out.sort_by(|a, b| b.played_at.cmp(&a.played_at)),
        TimeSort::OldestFirst => out.sort_by(|a, b| a.played_at.cmp(&b.played_at)),
    }
    out
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

/// What the unsure chip at the end of a row says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unsure {
    /// No outcome read: the chip opens the game to set one.
    NoOutcome,
    /// This many stats the reader was not sure of.
    Stats(usize),
}

impl Unsure {
    pub fn label(&self) -> String {
        match self {
            Unsure::NoOutcome => "⚠ No outcome · Set".into(),
            Unsure::Stats(1) => "⚠ 1 stat unsure".into(),
            Unsure::Stats(n) => format!("⚠ {n} stats unsure"),
        }
    }
}

/// The chip for `game`, if any. A missing outcome wins over unsure stats.
pub fn unsure(game: &Game) -> Option<Unsure> {
    if game.outcome == Outcome::Unknown {
        return Some(Unsure::NoOutcome);
    }
    let n = game.suspect_fields.len();
    (n > 0).then_some(Unsure::Stats(n))
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

/// "Sessions split after 3 hours without a game." from the session window.
pub fn split_note(gap_secs: u64) -> String {
    let mins = gap_secs / 60;
    let span = if mins >= 60 && mins % 60 == 0 {
        let h = mins / 60;
        if h == 1 {
            "1 hour".to_string()
        } else {
            format!("{h} hours")
        }
    } else if mins == 1 {
        "1 minute".to_string()
    } else {
        format!("{mins} minutes")
    };
    format!(
        "Sessions split after {span} without a game. Rank, queue, length and crew are not recorded yet."
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
    fn unsure_chip_labels() {
        let g = game(Outcome::Win, "2026-10-18T22:40:00Z", &["elims", "deaths"]);
        assert_eq!(unsure(&g).unwrap().label(), "⚠ 2 stats unsure");
        let g = game(Outcome::Loss, "2026-10-18T22:40:00Z", &["elims"]);
        assert_eq!(unsure(&g).unwrap().label(), "⚠ 1 stat unsure");
        let g = game(Outcome::Unknown, "2026-10-18T22:40:00Z", &["elims"]);
        assert_eq!(unsure(&g).unwrap().label(), "⚠ No outcome · Set");
        let g = game(Outcome::Win, "2026-10-18T22:40:00Z", &[]);
        assert_eq!(unsure(&g), None);
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
        assert_eq!(mode_chip(""), None);
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

    #[test]
    fn time_sort_flips_order() {
        let games = [
            game(Outcome::Win, "2026-10-18T00:14:00Z", &[]),
            game(Outcome::Loss, "2026-10-17T22:33:00Z", &[]),
        ];
        let refs: Vec<&Game> = games.iter().collect();
        let newest = sorted(&refs, TimeSort::NewestFirst);
        assert_eq!(newest[0].outcome, Outcome::Win);
        let oldest = sorted(&refs, TimeSort::OldestFirst);
        assert_eq!(oldest[0].outcome, Outcome::Loss);
        assert_eq!(TimeSort::NewestFirst.flipped(), TimeSort::OldestFirst);
        assert_eq!(TimeSort::NewestFirst.arrow(), "▼");
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
        assert!(split_note(10800).starts_with("Sessions split after 3 hours without a game."));
        assert!(split_note(1800).starts_with("Sessions split after 30 minutes"));
        assert!(split_note(3600).starts_with("Sessions split after 1 hour "));
        assert_eq!(filters_label(0), "Filters ▾");
        assert_eq!(filters_label(1), "Filters (1) ▾");
    }
}
