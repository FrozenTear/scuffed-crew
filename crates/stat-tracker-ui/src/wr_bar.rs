//! The 0.6 win-rate bar (spec r1 `common.mb`): a track with a 50% midline,
//! filled from the midline toward the win rate. A full half bar is 25
//! points (25% to 75%, capped beyond), any rate other than 50% gets at
//! least [`MIN_FILL`] px in its direction, and exactly 50% is the bare
//! midline. Mint above 50%, red below; the printed % sits next to it, so
//! the colour is never the only signal.
//!
//! Maps and Seasons draw it now. Overview and Heroes switch to it with
//! their redesigns, so every page shows one scale.

use iced::widget::{container, row, space, stack};
use iced::{Background, Border, Color, Element, Theme};

use crate::aggregate::Record;
use crate::app::Message;
use crate::theme::{self, TEXT_3};

/// Track colour (spec `--surface2`).
pub const SURFACE_2: Color = Color::from_rgb(
    0x23 as f32 / 255.0,
    0x23 as f32 / 255.0,
    0x2e as f32 / 255.0,
);
/// Points either side of 50% that fill a whole half.
pub const HALF_SPAN: f32 = 25.0;
/// Shortest fill for any rate other than 50%.
pub const MIN_FILL: f32 = 12.0;
/// A win rate needs this many games. Under it the row shows its record and
/// "Too few games", with no % and no bar.
pub const MIN_GAMES: usize = 10;
/// Copy shown in place of a win rate under [`MIN_GAMES`].
pub const TOO_FEW: &str = "Too few games";

const TRACK_H: f32 = 8.0;
/// The midline sticks out 2 px above and below the track.
const MID_H: f32 = TRACK_H + 4.0;

/// Which way the bar fills from the midline, and how far in px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BarFill {
    /// Exactly 50%: the midline alone.
    Even,
    Up(f32),
    Down(f32),
}

/// The fill for `pct` (0 to 100) on a bar `width` px wide.
pub fn fill(pct: u32, width: f32) -> BarFill {
    let pct = pct.min(100);
    if pct == 50 {
        return BarFill::Even;
    }
    let half = (width / 2.0).max(0.0);
    let off = (pct as f32 - 50.0).abs();
    let px = ((off / HALF_SPAN).min(1.0) * half).max(MIN_FILL).min(half);
    if pct > 50 {
        BarFill::Up(px)
    } else {
        BarFill::Down(px)
    }
}

/// The win rate a row prints: whole percent of decided games, or `None`
/// under [`MIN_GAMES`] games or with no decided game.
pub fn shown_pct(record: &Record) -> Option<u32> {
    let decided = record.wins + record.losses + record.draws;
    if record.games < MIN_GAMES || decided == 0 {
        return None;
    }
    Some(record.win_rate_pct().round() as u32)
}

/// Mint at 50% or above, red below (Best and Worst lists, printed %).
pub fn pct_color(pct: u32) -> Color {
    if pct >= 50 { theme::OK } else { theme::DANGER }
}

fn block(color: Color, w: f32, h: f32, radius: f32) -> Element<'static, Message> {
    container(space().width(w).height(h))
        .width(w)
        .height(h)
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

/// The bar for `pct`, `width` px wide.
pub fn bar(pct: u32, width: f32) -> Element<'static, Message> {
    let half = (width / 2.0).floor();
    let (left, right) = match fill(pct, width) {
        BarFill::Even => (None, None),
        BarFill::Up(px) => (None, Some(block(theme::OK, px, TRACK_H, 2.0))),
        BarFill::Down(px) => (Some(block(theme::DANGER, px, TRACK_H, 2.0)), None),
    };
    let side = |part: Option<Element<'static, Message>>, x: iced::alignment::Horizontal| {
        container(part.unwrap_or_else(|| space().width(0).into()))
            .width(half)
            .height(TRACK_H)
            .align_x(x)
    };
    let track = container(row![
        side(left, iced::alignment::Horizontal::Right),
        side(right, iced::alignment::Horizontal::Left),
    ])
    .width(half * 2.0)
    .height(TRACK_H)
    .style(|_: &Theme| container::Style {
        background: Some(Background::Color(SURFACE_2)),
        border: Border {
            radius: (TRACK_H / 2.0).into(),
            ..Border::default()
        },
        ..container::Style::default()
    });
    stack![
        container(track)
            .width(half * 2.0)
            .height(MID_H)
            .align_y(iced::alignment::Vertical::Center),
        container(block(TEXT_3, 1.0, MID_H, 0.0))
            .width(half * 2.0)
            .height(MID_H)
            .align_x(iced::alignment::Horizontal::Center),
    ]
    .width(half * 2.0)
    .height(MID_H)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(games: usize, wins: usize, losses: usize, draws: usize) -> Record {
        Record {
            games,
            wins,
            losses,
            draws,
        }
    }

    #[test]
    fn exactly_fifty_is_the_bare_midline() {
        assert_eq!(fill(50, 200.0), BarFill::Even);
    }

    #[test]
    fn a_half_bar_is_twenty_five_points_and_caps() {
        assert_eq!(fill(75, 200.0), BarFill::Up(100.0));
        assert_eq!(fill(78, 200.0), BarFill::Up(100.0));
        assert_eq!(fill(25, 200.0), BarFill::Down(100.0));
        assert_eq!(fill(20, 200.0), BarFill::Down(100.0));
        assert_eq!(fill(100, 200.0), BarFill::Up(100.0));
        assert_eq!(fill(0, 200.0), BarFill::Down(100.0));
        // 63%: 13 of 25 points of a 100 px half.
        assert_eq!(fill(63, 200.0), BarFill::Up(52.0));
    }

    #[test]
    fn near_fifty_still_gets_twelve_px() {
        assert_eq!(fill(49, 200.0), BarFill::Down(MIN_FILL));
        assert_eq!(fill(52, 200.0), BarFill::Up(MIN_FILL));
        assert_eq!(fill(53, 160.0), BarFill::Up(MIN_FILL));
        // Never past the half, even on a tiny bar.
        assert_eq!(fill(51, 16.0), BarFill::Up(8.0));
    }

    #[test]
    fn win_rate_needs_ten_games_and_a_decided_one() {
        assert_eq!(shown_pct(&rec(9, 9, 0, 0)), None);
        assert_eq!(shown_pct(&rec(10, 0, 0, 0)), None);
        // Undecided games count toward the 10 but not the rate.
        assert_eq!(shown_pct(&rec(62, 28, 22, 1)), Some(55));
        assert_eq!(shown_pct(&rec(110, 52, 55, 0)), Some(49));
        assert_eq!(shown_pct(&rec(12, 5, 5, 0)), Some(50));
    }

    #[test]
    fn colours_split_at_fifty() {
        assert_eq!(pct_color(50), theme::OK);
        assert_eq!(pct_color(49), theme::DANGER);
        let _ = bar(49, 200.0);
        let _ = bar(50, 160.0);
        let _ = bar(78, 140.0);
    }
}
