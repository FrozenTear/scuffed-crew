//! Draws the open Games row's "Bans and picks" block from the view model in
//! `games_bans`. Chips are read-only: no focus, no press. The one button is
//! "Open Settings" on a reason line.

use iced::widget::text::Wrapping;
use iced::widget::{Row, button, column, container, image, row, space, text, tooltip};
use iced::{Alignment, Background, Border, Element, Fill, Length, Padding, Theme};

use crate::app::Message;
use crate::games_bans::{BansPicks, Body, Chip, HEADER, Row as BlockRow, SettingsLink, Slot};
use crate::games_table::{pill_box, portrait};
use crate::icons::{self, Icon};
use crate::model::Game;
use crate::settings_model::AdvancedSection;
use crate::theme::{
    self, ACCENT, BG, BORDER, FONT_BOLD, FONT_SEMIBOLD, SIZE_LABEL, TEXT, TEXT_2, TEXT_3,
};

/// The text floor in this block (the label class is 11px).
const SMALL: f32 = 12.0;
const CHIP_H: f32 = 28.0;
/// Chip width at 1366 and up, and at 1024 (labels above the rows).
const CHIP_W: f32 = 156.0;
const CHIP_W_NARROW: f32 = 148.0;
/// The swap chip is wider, so a long name, "You" and the glyph fit.
const SWAP_EXTRA: f32 = 28.0;
const LABEL_TEAM_W: f32 = 96.0;
const LABEL_LINE_W: f32 = 84.0;

/// The block for a game, or `None` when nothing is to be drawn.
pub fn view(g: &Game, narrow: bool) -> Option<Element<'static, Message>> {
    let block = crate::games_bans::bans_picks(g);
    let rows = match &block {
        BansPicks::Hidden => return None,
        BansPicks::Reason {
            line,
            tip,
            settings,
        } => return Some(reason(line.clone(), *tip, *settings)),
        BansPicks::Block(rows) => rows,
    };
    let mut col = column![rule(), header(&block)].spacing(8);
    for r in rows {
        col = col.push(block_row(r, narrow));
    }
    Some(col.into())
}

fn rule() -> Element<'static, Message> {
    container(space().height(1))
        .width(Fill)
        .height(1)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(BORDER)),
            ..container::Style::default()
        })
        .into()
}

fn label() -> Element<'static, Message> {
    text(HEADER.to_uppercase())
        .size(SIZE_LABEL)
        .font(FONT_BOLD)
        .color(TEXT_3)
        .into()
}

/// The label, then a legend of the glyphs the block uses.
fn header(block: &BansPicks) -> Element<'static, Message> {
    let mut legend = row![].spacing(16).align_y(Alignment::Center);
    if block.has_ban() {
        legend = legend.push(legend_icon(Icon::Ban, "Banned"));
    }
    if block.has_swap() {
        legend = legend.push(legend_icon(
            Icon::Swap,
            "Played a different hero than at hero select",
        ));
    }
    if block.has_not_read() {
        legend = legend.push(
            row![
                container(space().width(14).height(10))
                    .width(14)
                    .height(10)
                    .style(not_read_swatch),
                text("Not read").size(SMALL).color(TEXT_3),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        );
    }
    row![label(), space().width(Fill), legend]
        .height(24)
        .align_y(Alignment::Center)
        .into()
}

fn legend_icon(icon: Icon, words: &'static str) -> Element<'static, Message> {
    row![
        image(icons::handle(icon)).width(12).height(12),
        text(words).size(SMALL).color(TEXT_3),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .into()
}

fn not_read_swatch(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: TEXT_3,
            width: 1.0,
            radius: 3.0.into(),
        },
        ..container::Style::default()
    }
}

/// One row: the label column and the chips (or the quiet line). At 1024 the
/// label goes above the chips and they get the full width.
fn block_row(r: &BlockRow, narrow: bool) -> Element<'static, Message> {
    let mut label = row![
        text(r.team)
            .size(SMALL)
            .font(FONT_SEMIBOLD)
            .color(TEXT_2)
            .wrapping(Wrapping::None),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if let Some(line) = r.line {
        label = label.push(
            text(line)
                .size(SMALL)
                .color(TEXT_3)
                .wrapping(Wrapping::None),
        );
    }
    if let Some(note) = r.note {
        label = label.push(
            text(note)
                .size(SMALL)
                .color(TEXT_3)
                .wrapping(Wrapping::None),
        );
    }
    let body: Element<'static, Message> = match &r.body {
        Body::Chips(chips) => {
            let mut chips_row: Row<'static, Message> = row![].spacing(8);
            for c in chips {
                chips_row = chips_row.push(chip(c, narrow));
            }
            chips_row.into()
        }
        Body::Quiet(line) => text(line.clone())
            .size(SMALL)
            .color(TEXT_3)
            .wrapping(Wrapping::None)
            .into(),
    };
    if narrow {
        column![label, body].spacing(4).into()
    } else {
        // The team and the line are two cells of a 180px label column; a
        // row with no line (Bans, Lobby ban) keeps the whole width.
        row![
            container(label)
                .width(LABEL_TEAM_W + LABEL_LINE_W)
                .height(CHIP_H)
                .align_y(Alignment::Center),
            container(body).height(CHIP_H).align_y(Alignment::Center),
        ]
        .into()
    }
}

/// One chip, with its tooltip. The tooltip is the spoken label too.
fn chip(c: &Chip, narrow: bool) -> Element<'static, Message> {
    let base = if narrow { CHIP_W_NARROW } else { CHIP_W };
    let width = if c.swap { base + SWAP_EXTRA } else { base };
    let not_read = c.slot == Slot::NotRead;
    let name_color = if c.ban || not_read { TEXT_2 } else { TEXT };
    let mut inner = row![].spacing(6).align_y(Alignment::Center);
    if c.ban {
        inner = inner.push(image(icons::handle(Icon::Ban)).width(12).height(12));
    }
    match &c.slot {
        Slot::Hero(_) => inner = inner.push(portrait(20.0, 12.0)),
        Slot::NoPreference => inner = inner.push(hollow_circle()),
        Slot::NotRead => {}
    }
    inner = inner.push(
        text(c.name().to_string())
            .size(SMALL)
            .font(FONT_SEMIBOLD)
            .color(name_color)
            .wrapping(Wrapping::None),
    );
    inner = inner.push(space().width(Fill));
    if c.you {
        inner = inner.push(text("You").size(SMALL).color(TEXT_3));
    }
    if c.swap {
        inner = inner.push(image(icons::handle(Icon::Swap)).width(12).height(12));
    }
    let ban = c.ban;
    let body = container(inner)
        .width(Length::Fixed(width))
        .height(CHIP_H)
        .padding(Padding::from([0, 8]))
        .align_y(Alignment::Center)
        .clip(true)
        .style(move |_: &Theme| container::Style {
            // A ban is transparent: removed, not bad. Not read is the same
            // plain outline in the dimmer grey.
            background: (!ban && !not_read).then_some(Background::Color(BG)),
            border: Border {
                color: if not_read { TEXT_3 } else { BORDER },
                width: 1.0,
                radius: 8.0.into(),
            },
            ..container::Style::default()
        });
    tooltip(
        body,
        container(text(c.tip.clone()).size(SMALL).color(TEXT))
            .padding(Padding::from([4, 8]))
            .style(pill_box(BORDER)),
        tooltip::Position::Top,
    )
    .gap(4)
    .into()
}

/// The hollow circle that stands in for hero art on a No preference chip.
fn hollow_circle() -> Element<'static, Message> {
    container(space().width(14).height(14))
        .width(20)
        .height(20)
        .center(20)
        .padding(3)
        .style(|_: &Theme| container::Style {
            border: Border {
                color: TEXT_3,
                width: 1.5,
                radius: 999.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// A game that read nothing and says why: the label, one quiet line that
/// never wraps, and "Open Settings" when the fix is there.
fn reason(
    line: String,
    tip: Option<&'static str>,
    settings: Option<SettingsLink>,
) -> Element<'static, Message> {
    let line: Element<'static, Message> = text(line)
        .size(SMALL)
        .color(TEXT_3)
        .wrapping(Wrapping::None)
        .into();
    let line = match tip {
        Some(tip) => tooltip(
            line,
            container(text(tip).size(SMALL).color(TEXT))
                .padding(Padding::from([4, 8]))
                .style(pill_box(BORDER)),
            tooltip::Position::Top,
        )
        .gap(4)
        .into(),
        None => line,
    };
    let mut r = row![label(), line].spacing(16).align_y(Alignment::Center);
    if let Some(link) = settings {
        r = r.push(settings_button(link));
    }
    column![rule(), container(r).height(28).align_y(Alignment::Center)]
        .spacing(12)
        .into()
}

/// What "Open Settings" sends: Settings with the row that holds the fix open
/// and in view.
fn settings_message(open: AdvancedSection) -> Message {
    Message::Nav(crate::nav::Msg::ShowAdvancedSection(open))
}

/// "Open Settings": an accent outline, so it is not colour alone.
fn settings_button(link: SettingsLink) -> Element<'static, Message> {
    let b = button(
        text("Open Settings")
            .size(SMALL)
            .font(FONT_SEMIBOLD)
            .color(ACCENT),
    )
    .padding(Padding::from([4, 10]))
    .style(|_: &Theme, status| button::Style {
        background: theme::hover_tint(status),
        text_color: ACCENT,
        border: Border {
            color: ACCENT,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..button::Style::default()
    })
    .on_press(settings_message(link.open));
    tooltip(
        b,
        container(text(link.tip).size(SMALL).color(TEXT))
            .padding(Padding::from([4, 8]))
            .max_width(320)
            .style(pill_box(BORDER)),
        tooltip::Position::Top,
    )
    .gap(4)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_settings_targets_the_row_that_holds_the_fix() {
        assert!(matches!(
            settings_message(AdvancedSection::Troubleshooting),
            Message::Nav(crate::nav::Msg::ShowAdvancedSection(
                AdvancedSection::Troubleshooting
            ))
        ));
        assert!(matches!(
            settings_message(AdvancedSection::WebsiteConnection),
            Message::Nav(crate::nav::Msg::ShowAdvancedSection(
                AdvancedSection::WebsiteConnection
            ))
        ));
    }
}
