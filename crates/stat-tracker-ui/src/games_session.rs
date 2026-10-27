//! A session on the 0.6 Games page (spec p2): its heading ("Sunday night ·
//! Oct 18, 23:31 to 00:52"), the win and loss sparkline beside it, and the
//! summary on the right ("5 games · 4-1 · 1 h 21 min"). Sessions split
//! after the Session window setting without a game.
//!
//! Small samples: a session shows its record, never a win rate.

use chrono::{DateTime, Datelike, Local, Timelike};
use iced::widget::{Column, Row, column, container, row, space, text};
use iced::{Alignment, Background, Element, Fill, Length, Theme};

use crate::aggregate::{Record, SessionGroup};
use crate::app::Message;
use crate::model::{Game, Outcome};
use crate::theme::{
    self, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_META, TEXT, TEXT_2, TEXT_3,
};

/// A sparkline bar: width, height, and the gap between bars.
const BAR_W: f32 = 3.0;
const BAR_H: f32 = 8.0;
const BAR_GAP: f32 = 2.0;
/// The sparkline's height: a win bar sits at the top, a loss at the bottom.
const SPARK_H: f32 = 13.0;

/// The part of the day a session started in. A start before 05:00 belongs
/// to the evening before ("Sunday night" for 00:30 on Monday).
pub fn day_part(start: DateTime<Local>) -> (chrono::NaiveDate, &'static str) {
    let hour = start.hour();
    let day = if hour < 5 {
        start.date_naive().pred_opt().unwrap_or(start.date_naive())
    } else {
        start.date_naive()
    };
    let part = match hour {
        5..=11 => "morning",
        12..=16 => "afternoon",
        17..=20 => "evening",
        _ => "night",
    };
    (day, part)
}

/// "Sunday night · Oct 18, 23:31 to 00:52" for a session from `first` to
/// `last` (its oldest and newest game). One game reads "…, 23:31".
pub fn heading(first: DateTime<Local>, last: DateTime<Local>) -> String {
    let (day, part) = day_part(first);
    let from = first.format("%H:%M");
    let when = format!(
        "{} {part} · {} {}",
        day.format("%A"),
        day.format("%b"),
        day.day()
    );
    if last <= first {
        format!("{when}, {from}")
    } else {
        format!("{when}, {from} to {}", last.format("%H:%M"))
    }
}

/// "1 h 21 min", "48 min", "under 1 min".
pub fn span_line(minutes: i64) -> String {
    match minutes {
        m if m < 1 => "under 1 min".to_string(),
        m if m < 60 => format!("{m} min"),
        m if m % 60 == 0 => format!("{} h", m / 60),
        m => format!("{} h {} min", m / 60, m % 60),
    }
}

/// The record without a rate (small-sample rule): "4-1", "3-3-1" with
/// draws, then undecided games as "· 1 undecided".
pub fn record_line(record: &Record) -> String {
    let undecided = record
        .games
        .saturating_sub(record.wins + record.losses + record.draws);
    let mut line = record.wl_label();
    if undecided > 0 {
        line.push_str(&format!(" · {undecided} undecided"));
    }
    line
}

/// The right-hand summary: games, record, and the time from the first game
/// to the last.
pub fn summary(group: &SessionGroup) -> String {
    let (first, last) = bounds(&group.games);
    let minutes = (last - first).num_minutes();
    format!(
        "{} · {} · {}",
        group.record.games_label(),
        record_line(&group.record),
        span_line(minutes)
    )
}

/// The oldest and newest game times, local. Groups hold newest first.
fn bounds(games: &[Game]) -> (DateTime<Local>, DateTime<Local>) {
    let local = |g: &Game| g.played_at.with_timezone(&Local);
    match (games.last(), games.first()) {
        (Some(oldest), Some(newest)) => (local(oldest), local(newest)),
        _ => (Local::now(), Local::now()),
    }
}

/// The heading row: title and sparkline on the left, the summary right.
pub fn header(group: &SessionGroup) -> Element<'static, Message> {
    let (first, last) = bounds(&group.games);
    Row::new()
        .spacing(12)
        .align_y(Alignment::Center)
        .push(
            text(heading(first, last))
                .size(SIZE_BODY)
                .font(FONT_BOLD)
                .color(TEXT),
        )
        .push(sparkline(&group.games))
        .push(space().width(Fill))
        .push(summary_text(group))
        .into()
}

/// "5 games · 4-1 · 1 h 21 min" with the record and length in bold.
fn summary_text(group: &SessionGroup) -> Element<'static, Message> {
    let line = summary(group);
    let mut parts = line.splitn(2, " · ");
    let games = parts.next().unwrap_or_default().to_string();
    let rest = parts.next().unwrap_or_default().to_string();
    row![
        text(format!("{games} · "))
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_2),
        text(rest).size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT),
    ]
    .into()
}

/// One bar per game, oldest on the left: a win is a mint bar at the top, a
/// loss a red bar at the bottom, a draw or undecided game a grey tick in
/// the middle.
pub fn sparkline(games: &[Game]) -> Element<'static, Message> {
    let mut bars = Row::new().spacing(BAR_GAP).height(SPARK_H);
    for g in games.iter().rev() {
        let (color, height, align) = match g.outcome {
            Outcome::Win => (theme::OK, BAR_H, Alignment::Start),
            Outcome::Loss => (theme::DANGER, BAR_H, Alignment::End),
            Outcome::Draw | Outcome::Unknown => (TEXT_3, 3.0, Alignment::Center),
        };
        let bar = container(space().width(BAR_W).height(height))
            .width(BAR_W)
            .height(height)
            .style(move |_: &Theme| container::Style {
                background: Some(Background::Color(color)),
                ..container::Style::default()
            });
        bars = bars.push(
            Column::new()
                .height(Length::Fixed(SPARK_H))
                .push(container(bar).height(Fill).align_y(align)),
        );
    }
    column![bars].into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(d: u32, h: u32, m: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, d, h, m, 0).unwrap()
    }

    #[test]
    fn a_session_after_midnight_belongs_to_the_night_before() {
        // Oct 18 2026 is a Sunday.
        assert_eq!(
            heading(at(18, 23, 31), at(19, 0, 52)),
            "Sunday night · Oct 18, 23:31 to 00:52"
        );
        assert_eq!(
            heading(at(19, 0, 10), at(19, 0, 40)),
            "Sunday night · Oct 18, 00:10 to 00:40"
        );
        assert_eq!(
            heading(at(17, 14, 5), at(17, 14, 5)),
            "Saturday afternoon · Oct 17, 14:05"
        );
    }

    #[test]
    fn spans_and_records_read_like_the_spec() {
        assert_eq!(span_line(81), "1 h 21 min");
        assert_eq!(span_line(120), "2 h");
        assert_eq!(span_line(9), "9 min");
        assert_eq!(span_line(0), "under 1 min");
        let record = Record {
            games: 7,
            wins: 3,
            losses: 3,
            draws: 0,
        };
        assert_eq!(record_line(&record), "3-3 · 1 undecided");
        let clean = Record {
            games: 5,
            wins: 4,
            losses: 1,
            draws: 0,
        };
        assert_eq!(record_line(&clean), "4-1");
    }

    #[test]
    fn no_session_line_shows_a_rate() {
        let record = Record {
            games: 3,
            wins: 2,
            losses: 1,
            draws: 0,
        };
        assert!(!record_line(&record).contains('%'));
    }
}
