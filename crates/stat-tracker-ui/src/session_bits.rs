//! Small pieces the 0.6 pages share about a run of games: the win and loss
//! sparkline (spec p1 and p2), "1 h 21 min", and which evening a session
//! belongs to ("Sunday night").

use chrono::{DateTime, Local, NaiveDate, Timelike};
use iced::widget::{Column, Row, container, space};
use iced::{Alignment, Background, Element, Fill, Length, Theme};

use crate::app::Message;
use crate::model::{Game, Outcome};
use crate::theme::{self, TEXT_3};

/// A sparkline bar: width, height, the gap between bars, and the line's
/// height (a win bar sits at the top, a loss bar at the bottom).
const BAR_W: f32 = 3.0;
const BAR_H: f32 = 8.0;
const BAR_GAP: f32 = 2.0;
const SPARK_H: f32 = 13.0;

/// The day and part of the day a session started in. A start before 05:00
/// belongs to the evening before ("Sunday night" for 00:30 on Monday).
pub fn day_part(start: DateTime<Local>) -> (NaiveDate, &'static str) {
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

/// "1 h 21 min", "48 min", "under 1 min".
pub fn span_line(minutes: i64) -> String {
    match minutes {
        m if m < 1 => "under 1 min".to_string(),
        m if m < 60 => format!("{m} min"),
        m if m % 60 == 0 => format!("{} h", m / 60),
        m => format!("{} h {} min", m / 60, m % 60),
    }
}

/// One bar per game in `oldest_first` order: a win is a mint bar at the
/// top, a loss a red bar at the bottom, a draw or undecided game a grey
/// tick in the middle.
pub fn sparkline<'a>(
    oldest_first: impl IntoIterator<Item = &'a Game>,
) -> Element<'static, Message> {
    outcome_sparkline(oldest_first.into_iter().map(|g| g.outcome))
}

/// [`sparkline`] from the outcomes alone (the Seasons Last 20 tile).
pub fn outcome_sparkline(
    oldest_first: impl IntoIterator<Item = Outcome>,
) -> Element<'static, Message> {
    let mut bars = Row::new().spacing(BAR_GAP).height(SPARK_H);
    for outcome in oldest_first {
        let (color, height, align) = match outcome {
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
    bars.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn after_midnight_is_the_night_before() {
        let late = Local.with_ymd_and_hms(2026, 10, 19, 0, 30, 0).unwrap();
        let (day, part) = day_part(late);
        assert_eq!(day.to_string(), "2026-10-18");
        assert_eq!(part, "night");
        let noon = Local.with_ymd_and_hms(2026, 10, 19, 13, 0, 0).unwrap();
        assert_eq!(day_part(noon).1, "afternoon");
    }

    #[test]
    fn spans_read_like_the_spec() {
        assert_eq!(span_line(81), "1 h 21 min");
        assert_eq!(span_line(120), "2 h");
        assert_eq!(span_line(9), "9 min");
        assert_eq!(span_line(0), "under 1 min");
    }
}
