//! Draws the open Games row's "Bans and picks" block from the view model in
//! `games_bans` (UX Spec bans and picks a3b, "chips stay default"). The
//! header is 24px: the label, the amber pill when there is one, then a
//! one-line legend of the glyphs this card uses. Under it, four rows 26px
//! high with 6px gaps, each with its label on the left at every width (the
//! 1024 layout too). Chips are 26px high and as wide as their name, with no
//! placeholder portrait. Chips are read-only: no focus, no press. The one
//! button is "Open Settings" on a reason line.
//!
//! Glyphs are the PNG icons, never text: Urbanist, the page font, has no
//! warning sign and no swap arrow glyph, so a text glyph would draw as a
//! missing-glyph box. The ban mark is `Icon::Ban`, the swap mark
//! `Icon::Swap`, and the unsure mark `Icon::Warn` (warn.png is painted in
//! the theme's WARN amber).

use iced::widget::text::Wrapping;
use iced::widget::{Row, button, column, container, image, row, space, text, tooltip};
use iced::{Alignment, Background, Border, Element, Fill, Padding, Theme};

use crate::app::Message;
use crate::games_bans::{
    BansPicks, Block, Chip, HEADER, LEGEND_BANNED, LEGEND_OUTLINE, LEGEND_SWAP, LEGEND_UNSURE,
    Legend, Part, Pill, Row as BlockRow, SettingsLink, Slot,
};
use crate::games_table::pill_box;
use crate::icons::{self, Icon};
use crate::model::Game;
use crate::settings_model::AdvancedSection;
use crate::theme::{
    self, ACCENT, BORDER, FONT_BOLD, FONT_SEMIBOLD, SIZE_LABEL, SURFACE_2, TEXT, TEXT_2, TEXT_3,
    WARN,
};

/// The text floor in this block (the label class is 11px).
const SMALL: f32 = 12.0;
const HEADER_H: f32 = 24.0;
const CHIP_H: f32 = 26.0;
const ROW_GAP: f32 = 6.0;
/// The row label column, left of the chips at every width.
const LABEL_W: f32 = 104.0;
const LABEL_GAP: f32 = 12.0;
const CHIP_GAP: f32 = 6.0;
const GLYPH: f32 = 12.0;
/// The space either side of a group divider.
const DIVIDER_GAP: f32 = 14.0;
const DIVIDER_H: f32 = 18.0;

/// The block for a game, or `None` when nothing is to be drawn. The layout
/// is the same at every width.
pub fn view(g: &Game) -> Option<Element<'static, Message>> {
    let block = match crate::games_bans::bans_picks(g) {
        BansPicks::Hidden => return None,
        BansPicks::Reason {
            line,
            tip,
            settings,
        } => return Some(reason(line, tip, settings)),
        BansPicks::Block(block) => block,
    };
    let mut rows = column![].spacing(ROW_GAP);
    for r in &block.rows {
        rows = rows.push(block_row(r));
    }
    Some(column![rule(), header(&block), rows].spacing(8).into())
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

fn tip_box(words: String) -> Element<'static, Message> {
    container(text(words).size(SMALL).color(TEXT))
        .padding(Padding::from([4, 8]))
        .max_width(360)
        .style(pill_box(BORDER))
        .into()
}

fn with_tip(
    body: impl Into<Element<'static, Message>>,
    words: String,
) -> Element<'static, Message> {
    tooltip(body, tip_box(words), tooltip::Position::Top)
        .gap(4)
        .into()
}

/// The label, the amber pill when there is one, then the legend.
fn header(block: &Block) -> Element<'static, Message> {
    let mut h = row![label()].spacing(12).align_y(Alignment::Center);
    if let Some(p) = block.pill() {
        h = h.push(pill(p));
    }
    let legend = block.legend();
    if !legend.is_empty() {
        h = h.push(container(legend_row(legend)).padding(Padding {
            left: 8.0,
            ..Padding::ZERO
        }));
    }
    container(h)
        .height(HEADER_H)
        .align_y(Alignment::Center)
        .into()
}

/// "N unsure" or "Reads may be off": an amber outline with the warning sign,
/// so it is not colour alone. Amber means unsure, nothing else.
fn pill(p: Pill) -> Element<'static, Message> {
    let body = container(
        row![
            image(icons::handle(Icon::Warn)).width(GLYPH).height(GLYPH),
            text(p.words())
                .size(SMALL)
                .font(FONT_SEMIBOLD)
                .color(WARN)
                .wrapping(Wrapping::None),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .height(HEADER_H)
    .padding(Padding::from([0, 10]))
    .align_y(Alignment::Center)
    .style(|_: &Theme| container::Style {
        border: Border {
            color: WARN,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..container::Style::default()
    });
    with_tip(body, p.tip().to_string())
}

/// One line, only the glyphs this card uses.
fn legend_row(legend: Legend) -> Element<'static, Message> {
    let mut r = row![].spacing(14).align_y(Alignment::Center);
    if legend.banned {
        r = r.push(legend_icon(Icon::Ban, LEGEND_BANNED));
    }
    if legend.unsure {
        r = r.push(legend_icon(Icon::Warn, LEGEND_UNSURE));
    }
    if legend.swap {
        r = r.push(legend_icon(Icon::Swap, LEGEND_SWAP));
    }
    if legend.outline {
        r = r.push(
            row![
                container(space().width(16).height(11))
                    .width(16)
                    .height(11)
                    .style(outline_swatch),
                legend_words(LEGEND_OUTLINE),
            ]
            .spacing(5)
            .align_y(Alignment::Center),
        );
    }
    r.into()
}

fn legend_words(words: &'static str) -> Element<'static, Message> {
    text(words)
        .size(SMALL)
        .color(TEXT_3)
        .wrapping(Wrapping::None)
        .into()
}

fn legend_icon(icon: Icon, words: &'static str) -> Element<'static, Message> {
    row![
        image(icons::handle(icon)).width(GLYPH).height(GLYPH),
        legend_words(words),
    ]
    .spacing(5)
    .align_y(Alignment::Center)
    .into()
}

fn outline_swatch(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: TEXT_3,
            width: 1.0,
            radius: 3.0.into(),
        },
        ..container::Style::default()
    }
}

/// One row: the label on the left, then the parts on one line.
fn block_row(r: &BlockRow) -> Element<'static, Message> {
    let mut body: Row<'static, Message> = row![].align_y(Alignment::Center);
    let mut prev: Option<&Part> = None;
    for part in &r.parts {
        if let Some(prev) = prev {
            body = body.push(space().width(gap(prev, part)));
        }
        body = body.push(draw_part(part));
        prev = Some(part);
    }
    row![
        container(
            text(r.label)
                .size(SMALL)
                .font(FONT_SEMIBOLD)
                .color(TEXT_3)
                .wrapping(Wrapping::None),
        )
        .width(LABEL_W),
        body,
    ]
    .spacing(LABEL_GAP)
    .height(CHIP_H)
    .align_y(Alignment::Center)
    .into()
}

/// The space between two parts of a row.
fn gap(prev: &Part, next: &Part) -> f32 {
    match (prev, next) {
        (Part::Divider, _) | (_, Part::Divider) => DIVIDER_GAP,
        (Part::Chips(_), Part::Note { .. }) => 10.0,
        _ => 8.0,
    }
}

fn draw_part(part: &Part) -> Element<'static, Message> {
    match part {
        Part::Key(key) => text(*key)
            .size(SMALL)
            .font(FONT_SEMIBOLD)
            .color(TEXT_3)
            .wrapping(Wrapping::None)
            .into(),
        Part::Chips(chips) => {
            let mut r: Row<'static, Message> = row![].spacing(CHIP_GAP);
            for c in chips {
                r = r.push(chip(c));
            }
            r.into()
        }
        Part::Note { text: words, tip } => with_tip(
            text(*words)
                .size(SMALL)
                .font(FONT_SEMIBOLD)
                .color(TEXT_3)
                .wrapping(Wrapping::None),
            (*tip).to_string(),
        ),
        Part::Quiet { text: words, tip } => {
            let line = text(*words)
                .size(SMALL)
                .color(TEXT_2)
                .wrapping(Wrapping::None);
            match tip {
                Some(tip) => with_tip(line, (*tip).to_string()),
                None => line.into(),
            }
        }
        Part::Divider => container(space().width(1).height(DIVIDER_H))
            .width(1)
            .height(DIVIDER_H)
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(BORDER)),
                ..container::Style::default()
            })
            .into(),
    }
}

/// One chip, as wide as its name, with its tooltip (the spoken label too).
/// A read hero is bold TEXT on the raised fill; a ban has no fill and TEXT_2 with the
/// ban glyph; Not read and Not seen are the same plain outline in the dimmer
/// grey; Skipped and No preference are neutral filled chips in TEXT_2. An
/// unsure read gets an amber outline and the warning sign. No red.
fn chip(c: &Chip) -> Element<'static, Message> {
    let hero = matches!(c.slot, Slot::Hero(_));
    let outline = c.outline();
    let unsure = c.unsure();
    let (font, color) = if outline || !hero {
        (FONT_SEMIBOLD, TEXT_2)
    } else if c.ban {
        (FONT_BOLD, TEXT_2)
    } else {
        (FONT_BOLD, TEXT)
    };
    let mut inner = row![].spacing(6).align_y(Alignment::Center);
    if c.ban && hero {
        inner = inner.push(image(icons::handle(Icon::Ban)).width(GLYPH).height(GLYPH));
    }
    if c.slot == Slot::NoPreference {
        inner = inner.push(hollow_circle());
    }
    inner = inner.push(
        text(c.name().to_string())
            .size(SMALL)
            .font(font)
            .color(color)
            .wrapping(Wrapping::None),
    );
    if c.you {
        inner = inner.push(
            text("You")
                .size(SMALL)
                .font(FONT_SEMIBOLD)
                .color(TEXT_3)
                .wrapping(Wrapping::None),
        );
    }
    if c.swap {
        inner = inner.push(image(icons::handle(Icon::Swap)).width(GLYPH).height(GLYPH));
    }
    if unsure {
        inner = inner.push(image(icons::handle(Icon::Warn)).width(GLYPH).height(GLYPH));
    }
    let filled = !c.ban && !outline;
    let body = container(inner)
        .height(CHIP_H)
        .padding(Padding::from([0, 9]))
        .align_y(Alignment::Center)
        .style(move |_: &Theme| container::Style {
            background: filled.then_some(Background::Color(SURFACE_2)),
            border: Border {
                color: if unsure {
                    WARN
                } else if outline {
                    TEXT_3
                } else {
                    BORDER
                },
                width: 1.0,
                radius: 8.0.into(),
            },
            ..container::Style::default()
        });
    with_tip(body, c.tooltip())
}

/// The hollow circle that stands in for hero art on a No preference chip.
fn hollow_circle() -> Element<'static, Message> {
    container(space().width(GLYPH).height(GLYPH))
        .width(GLYPH)
        .height(GLYPH)
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
        Some(tip) => with_tip(line, tip.to_string()),
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
