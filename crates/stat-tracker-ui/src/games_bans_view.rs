//! Draws the open Games row's "Bans and picks" block from the view model in
//! `games_bans` (UX Spec bans and picks a3b, "chips stay default"). The
//! header is 24px: the label, then a one-line legend of the glyphs this card
//! uses. Under it, four rows 26px high with 6px gaps, each with its label on
//! the left at every width (the 1024 layout too). Chips are 26px high and as
//! wide as their name, with no placeholder portrait. Chips are read-only: no
//! focus, no press. The one button is "Open Settings" on a reason line.
//!
//! Glyphs are the PNG icons, never text: Urbanist, the page font, has no
//! swap arrow glyph, so a text glyph would draw as a missing-glyph box. The
//! ban mark is `Icon::Ban` and the swap mark is `Icon::Swap`. cv-v1 has no
//! unsure state yet, so nothing amber is drawn: no warning sign, no "N
//! unsure" / "Reads may be off" pill, and the legend leaves out "Might be
//! misread".

use iced::widget::text::Wrapping;
use iced::widget::{Row, button, column, container, image, row, space, text, tooltip};
use iced::{Alignment, Background, Border, Color, Element, Fill, Padding, Theme};

use crate::app::Message;
use crate::games_bans::{
    BansPicks, Block, Chip, HEADER, Legend, LegendMark, Part, Row as BlockRow, SettingsLink, Slot,
};
use crate::games_table::pill_box;
use crate::icons::{self, Icon};
use crate::model::Game;
use crate::settings_model::AdvancedSection;
use crate::theme::{
    self, ACCENT, BORDER, FONT_BOLD, FONT_SEMIBOLD, SIZE_LABEL, SURFACE_2, TEXT, TEXT_2, TEXT_3,
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

/// The label, then the legend (no amber pill: cv-v1 has no unsure state).
fn header(block: &Block) -> Element<'static, Message> {
    let mut h = row![label()].spacing(12).align_y(Alignment::Center);
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

/// One line, only the glyphs this card uses.
fn legend_row(legend: Legend) -> Element<'static, Message> {
    let mut r = row![].spacing(14).align_y(Alignment::Center);
    for (mark, words) in legend.items() {
        r = r.push(match mark {
            LegendMark::Ban => legend_icon(Icon::Ban, words),
            LegendMark::Swap => legend_icon(Icon::Swap, words),
            LegendMark::Outline => row![
                container(space().width(16).height(11))
                    .width(16)
                    .height(11)
                    .style(outline_swatch),
                legend_words(words),
            ]
            .spacing(5)
            .align_y(Alignment::Center)
            .into(),
        });
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

/// How a chip is painted: its fill (none for bans and outlines), border and
/// text colour, and whether the name is bold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ChipLook {
    pub fill: Option<Color>,
    pub border: Color,
    pub text: Color,
    pub bold: bool,
}

/// The look of one chip. A read hero is bold TEXT on the raised fill; a ban
/// has no fill and TEXT_2 with the ban glyph; Not read and Not seen are the
/// same plain outline in the dimmer grey; Skipped and No preference are
/// neutral filled chips in TEXT_2. No amber and no red: cv-v1 has no unsure
/// state yet.
pub(crate) fn chip_look(c: &Chip) -> ChipLook {
    let hero = matches!(c.slot, Slot::Hero(_));
    let outline = c.outline();
    let (bold, text) = if outline || !hero {
        (false, TEXT_2)
    } else if c.ban {
        (true, TEXT_2)
    } else {
        (true, TEXT)
    };
    ChipLook {
        fill: (!c.ban && !outline).then_some(SURFACE_2),
        border: if outline { TEXT_3 } else { BORDER },
        text,
        bold,
    }
}

/// One chip, as wide as its name, with its tooltip (the spoken label too),
/// painted as [`chip_look`] says.
fn chip(c: &Chip) -> Element<'static, Message> {
    let look = chip_look(c);
    let mut inner = row![].spacing(6).align_y(Alignment::Center);
    if c.ban && matches!(c.slot, Slot::Hero(_)) {
        inner = inner.push(image(icons::handle(Icon::Ban)).width(GLYPH).height(GLYPH));
    }
    if c.slot == Slot::NoPreference {
        inner = inner.push(hollow_circle());
    }
    inner = inner.push(
        text(c.name().to_string())
            .size(SMALL)
            .font(if look.bold { FONT_BOLD } else { FONT_SEMIBOLD })
            .color(look.text)
            .wrapping(Wrapping::None),
    );
    if c.you_tag {
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
    let body = container(inner)
        .height(CHIP_H)
        .padding(Padding::from([0, 9]))
        .align_y(Alignment::Center)
        .style(move |_: &Theme| container::Style {
            background: look.fill.map(Background::Color),
            border: Border {
                color: look.border,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..container::Style::default()
        });
    with_tip(body, c.tip.clone())
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
    use crate::fixtures_games::{LOBBY_ONLY, NO_BAN, NORMAL, SIDE, SKIP, UNSURE};
    use crate::snapshot::games_from_snapshot;

    /// Every chip of every fixture block, plus the states no fixture has.
    fn all_chips() -> Vec<Chip> {
        let games = games_from_snapshot(&crate::fixtures_games::snapshot());
        let mut out: Vec<Chip> = Vec::new();
        for sid in [NORMAL, UNSURE, SIDE, NO_BAN, SKIP, LOBBY_ONLY] {
            let g = games.iter().find(|g| g.session_id == sid).expect(sid);
            let BansPicks::Block(b) = crate::games_bans::bans_picks(g) else {
                panic!("{sid}: no block");
            };
            for r in &b.rows {
                for p in &r.parts {
                    if let Part::Chips(c) = p {
                        out.extend(c.iter().cloned());
                    }
                }
            }
        }
        out
    }

    /// The plain outline of Not read and Not seen: no fill, the dim border.
    fn is_outline(look: ChipLook) -> bool {
        look.fill.is_none() && look.border == TEXT_3
    }

    /// No chip is ever amber or red, and only Not read and Not seen are the
    /// outline.
    #[test]
    fn chips_are_never_amber_and_only_unread_ones_are_outlines() {
        let chips = all_chips();
        for slot in [
            Slot::NotRead,
            Slot::Empty,
            Slot::Skipped,
            Slot::NoPreference,
        ] {
            assert!(chips.iter().any(|c| c.slot == slot), "{slot:?} not covered");
        }
        assert!(
            chips
                .iter()
                .any(|c| c.ban && matches!(c.slot, Slot::Hero(_)))
        );
        assert!(
            chips
                .iter()
                .any(|c| !c.ban && matches!(c.slot, Slot::Hero(_)))
        );
        let warm = [theme::WARN, theme::WARN_LINE, theme::DANGER];
        for c in &chips {
            let look = chip_look(c);
            let colours = [Some(look.border), Some(look.text), look.fill];
            assert!(
                colours.iter().flatten().all(|col| !warm.contains(col)),
                "{}: {look:?}",
                c.tip
            );
            assert_eq!(
                is_outline(look),
                matches!(c.slot, Slot::NotRead | Slot::Empty),
                "{}: {look:?}",
                c.tip
            );
        }
        // The four looks, pinned.
        let not_read = chips.iter().find(|c| c.slot == Slot::NotRead).unwrap();
        assert_eq!(
            chip_look(not_read),
            ChipLook {
                fill: None,
                border: TEXT_3,
                text: TEXT_2,
                bold: false
            }
        );
        let ban = chips
            .iter()
            .find(|c| c.ban && matches!(c.slot, Slot::Hero(_)))
            .unwrap();
        assert_eq!(
            chip_look(ban),
            ChipLook {
                fill: None,
                border: BORDER,
                text: TEXT_2,
                bold: true
            }
        );
        let hero = chips
            .iter()
            .find(|c| !c.ban && matches!(c.slot, Slot::Hero(_)))
            .unwrap();
        assert_eq!(
            chip_look(hero),
            ChipLook {
                fill: Some(SURFACE_2),
                border: BORDER,
                text: TEXT,
                bold: true
            }
        );
        let skipped = chips.iter().find(|c| c.slot == Slot::Skipped).unwrap();
        assert_eq!(
            chip_look(skipped),
            ChipLook {
                fill: Some(SURFACE_2),
                border: BORDER,
                text: TEXT_2,
                bold: false
            }
        );
    }

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
