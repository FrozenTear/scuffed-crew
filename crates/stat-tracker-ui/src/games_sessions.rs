//! Session headers for the Games table: the sittings (games grouped by
//! the gap between them), each sitting's title and summary, and its form
//! strip. Pure, like [`crate::games_rows`], which re-exports these and
//! lays the sections out.

use chrono::{DateTime, NaiveDate, TimeZone, Timelike, Utc};

use crate::model::{Game, Outcome, game_card_in_progress};

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
    // To the minute, as the title prints them, so "23:31 to 00:52" and
    // its span agree.
    let minute = |t: DateTime<Utc>| t.with_second(0).and_then(|t| t.with_nanosecond(0));
    let first = games.iter().map(|g| g.started_at()).min().and_then(minute);
    let last = games.iter().map(|g| g.ended_at()).max().and_then(minute);
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
/// between one game's end and the next game's start
/// ([`crate::aggregate::same_sitting`], the Overview's rule too). Inside
/// each, newest first.
pub fn sittings(games: &[Game], gap: chrono::TimeDelta) -> Vec<Vec<&Game>> {
    let mut newest: Vec<&Game> = games.iter().collect();
    newest.sort_by_key(|g| std::cmp::Reverse(g.started_at()));
    let mut out: Vec<Vec<&Game>> = Vec::new();
    for g in newest {
        match out.last_mut() {
            Some(cur)
                if cur
                    .last()
                    .is_some_and(|newer| crate::aggregate::same_sitting(newer, g, gap)) =>
            {
                cur.push(g);
            }
            _ => out.push(vec![g]),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games_rows::tests::{game, timed, utc};
    use chrono::TimeDelta;

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

    /// The span is taken from the times the title prints, to the minute:
    /// 23:31:40 to 00:52:10 reads "23:31 to 00:52" and "1 h 21 min", not
    /// "1 h 20 min".
    #[test]
    fn the_span_agrees_with_the_title_to_the_minute() {
        let games = [
            timed(Outcome::Win, "2026-10-19T00:40:00Z", "2026-10-19T00:52:10Z"),
            timed(Outcome::Win, "2026-10-18T23:31:40Z", "2026-10-18T23:50:00Z"),
        ];
        let groups = sittings(&games, TimeDelta::hours(3));
        let head = session_head_in(&groups[0], 2, None, &utc());
        assert_eq!(head.title, "Sunday night · Oct 18, 23:31 to 00:52");
        assert_eq!(head.summary, "2 games · 2-0 · 1 h 21 min");
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
    /// 20-minute game and a 10-minute queue stay one sitting under a 15
    /// minute window, and the span covers the last game's length. Only
    /// that rule passes at 15 minutes: start to start is 30 minutes, and
    /// last Tab read to last Tab read is 30 too.
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
        let groups = sittings(&games, TimeDelta::minutes(15));
        assert_eq!(groups.iter().map(Vec::len).collect::<Vec<_>>(), [2, 1]);
        let head = session_head_in(&groups[0], 2, None, &utc());
        assert_eq!(head.title, "Sunday evening · Oct 18, 20:00 to 20:50");
        assert_eq!(head.summary, "2 games · 1-1 · 50 min");
    }
}
