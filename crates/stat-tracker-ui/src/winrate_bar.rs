//! The one win-rate bar every 0.6 page draws (spec r1.1, `common.mb`):
//! an 8 px track with a midline at 50%. A rate above 50% fills to the
//! right in the win colour, below 50% to the left in the loss colour. Half
//! the track is 25 points (25% to 75%, capped beyond), any rate other than
//! 50% gets at least 12 px in its direction, and exactly 50% is the bare
//! midline. The Overview uses it; the other 0.6 pages (Heroes, Maps,
//! Seasons) are meant to share it.
//!
//! Callers draw it only from 10 games (small-sample rule).

use iced::widget::{container, responsive, row, space, stack};
use iced::{Background, Border, Element, Fill, Length, Theme};

use crate::app::Message;
use crate::theme::{self, BORDER, TEXT_3};

/// Track height and corner.
const TRACK_H: f32 = 8.0;
/// The midline sticks out this far above and below the track.
const TICK_OVER: f32 = 2.0;
/// Points of win rate that fill half the track.
const HALF_POINTS: f32 = 25.0;
/// Shortest fill for any rate other than 50%.
const MIN_FILL: f32 = 12.0;

/// The fill for `percent` on a track `width` px wide: its length in px,
/// positive to the right (wins), negative to the left, 0 at exactly 50%.
pub fn fill_px(percent: f32, width: f32) -> f32 {
    let half = (width / 2.0).max(0.0);
    let off = percent - 50.0;
    if off == 0.0 || !off.is_finite() {
        return 0.0;
    }
    let len = ((off.abs() / HALF_POINTS).min(1.0) * half)
        .max(MIN_FILL)
        .min(half);
    len.copysign(off)
}

fn block(width: f32, height: f32, color: iced::Color, radius: f32) -> Element<'static, Message> {
    container(space())
        .width(width)
        .height(height)
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(color)),
            border: Border {
                radius: radius.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// The bar for `percent` (0 to 100), as wide as its column.
pub fn bar(percent: f32) -> Element<'static, Message> {
    let height = TRACK_H + 2.0 * TICK_OVER;
    responsive(move |size| {
        let width = size.width;
        let half = width / 2.0;
        let fill = fill_px(percent, width);
        let track = container(block(width, TRACK_H, BORDER, TRACK_H / 2.0))
            .height(height)
            .center_y(height);
        let (pad, color) = if fill >= 0.0 {
            (half, theme::OK)
        } else {
            (half + fill, theme::DANGER)
        };
        let filled = container(row![
            space().width(pad.max(0.0)),
            block(fill.abs(), TRACK_H, color, 2.0),
        ])
        .height(height)
        .center_y(height);
        let tick = row![
            space().width((half - 0.5).max(0.0)),
            block(1.0, height, TEXT_3, 0.0),
        ];
        stack![track, filled, tick]
            .width(Fill)
            .height(height)
            .into()
    })
    .width(Fill)
    .height(Length::Fixed(height))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_the_track_is_twenty_five_points_and_capped() {
        assert_eq!(fill_px(75.0, 200.0), 100.0);
        assert_eq!(fill_px(100.0, 200.0), 100.0);
        assert_eq!(fill_px(25.0, 200.0), -100.0);
        assert_eq!(fill_px(0.0, 200.0), -100.0);
        assert_eq!(fill_px(62.5, 200.0), 50.0);
    }

    #[test]
    fn near_fifty_gets_twelve_px_and_fifty_is_a_bare_tick() {
        assert_eq!(fill_px(52.0, 200.0), 12.0);
        assert_eq!(fill_px(49.0, 200.0), -12.0);
        assert_eq!(fill_px(50.0, 200.0), 0.0);
    }

    #[test]
    fn a_narrow_track_never_overflows_its_half() {
        assert_eq!(fill_px(51.0, 16.0), 8.0);
        assert_eq!(fill_px(f32::NAN, 200.0), 0.0);
    }
}
