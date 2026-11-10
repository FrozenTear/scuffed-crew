//! Draws the open Games row's "Bans and picks" block from the view model in
//! `games_bans` (UX Spec bans and picks a3b, "chips stay default"). The
//! header is 24px: the label, then a one-line legend of the glyphs this card
//! uses. Under it, four rows 26px high with 6px gaps, each with its label on
//! the left at every width (the 1024 layout too). A row too wide for the
//! panel wraps between its groups onto a second line, 6px below. Chips are
//! 26px high and as wide as their name, with no placeholder portrait.
//! Chips are read-only: no focus, no press. The one button is "Open
//! Settings" on a reason line.
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
#[cfg(test)]
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

/// One row: the label on the left, then the parts. The label sits level
/// with the first line of the body.
fn block_row(r: &BlockRow) -> Element<'static, Message> {
    row![
        container(
            text(r.label)
                .size(SMALL)
                .font(FONT_SEMIBOLD)
                .color(TEXT_3)
                .wrapping(Wrapping::None),
        )
        .width(LABEL_W)
        .height(CHIP_H)
        .align_y(Alignment::Center),
        row_body(&r.parts),
    ]
    .spacing(LABEL_GAP)
    .align_y(Alignment::Start)
    .into()
}

/// The parts of a row on one line, wrapping onto more lines when the panel
/// is too narrow (a mixed Bans row with long names at 1024). The pieces
/// that wrap are the groups, each with the divider after it, so a new line
/// always starts with a group, never with a divider. Every line is at least
/// a chip high, with its parts centred in it.
fn row_body(parts: &[Part]) -> Element<'static, Message> {
    // The spacer keeps its default Shrink width: iced skips a child that is
    // `.width(0)`, which would leave a line of quiet text only as tall as
    // its words and above its label.
    let line =
        || -> Row<'static, Message> { row![space().height(CHIP_H)].align_y(Alignment::Center) };
    let mut body: Row<'static, Message> = row![].align_y(Alignment::Center);
    let mut unit = line();
    let mut prev: Option<&Part> = None;
    for part in parts {
        if let Some(prev) = prev {
            unit = unit.push(space().width(gap(prev, part)));
        }
        unit = unit.push(draw_part(part));
        prev = Some(part);
        if matches!(part, Part::Divider) {
            unit = unit.push(space().width(DIVIDER_GAP));
            body = body.push(std::mem::replace(&mut unit, line()));
            prev = None;
        }
    }
    body.push(unit).wrap().vertical_spacing(ROW_GAP).into()
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

/// What "Open Settings" sends: Settings with the section that holds the
/// fix open. A reader-off game also jumps to the Read bans and picks row.
fn settings_message(link: SettingsLink) -> Message {
    if link.reveal_row {
        Message::Nav(crate::nav::Msg::ShowBansPicksSetting)
    } else {
        Message::Nav(crate::nav::Msg::ShowAdvancedSection(link.open))
    }
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
    .on_press(settings_message(link));
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
    use crate::fixtures_games::{LOBBY_ONLY, MIXED, NO_BAN, NORMAL, SIDE, SKIP, UNSURE};
    use crate::snapshot::games_from_snapshot;

    /// Every chip of every fixture block.
    fn all_chips() -> Vec<Chip> {
        let games = games_from_snapshot(&crate::fixtures_games::snapshot());
        let mut out: Vec<Chip> = Vec::new();
        for sid in [NORMAL, UNSURE, SIDE, NO_BAN, SKIP, LOBBY_ONLY, MIXED] {
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
        let section = |open, reveal_row| SettingsLink {
            tip: "",
            open,
            reveal_row,
        };
        assert!(matches!(
            settings_message(section(AdvancedSection::Troubleshooting, false)),
            Message::Nav(crate::nav::Msg::ShowAdvancedSection(
                AdvancedSection::Troubleshooting
            ))
        ));
        assert!(matches!(
            settings_message(section(AdvancedSection::WebsiteConnection, false)),
            Message::Nav(crate::nav::Msg::ShowAdvancedSection(
                AdvancedSection::WebsiteConnection
            ))
        ));
        assert!(matches!(
            settings_message(section(AdvancedSection::CaptureReading, true)),
            Message::Nav(crate::nav::Msg::ShowBansPicksSetting)
        ));
        // Pack failed opens the section only, no jump to the switch.
        assert!(matches!(
            settings_message(section(AdvancedSection::CaptureReading, false)),
            Message::Nav(crate::nav::Msg::ShowAdvancedSection(
                AdvancedSection::CaptureReading
            ))
        ));
    }

    fn headless() -> iced::Renderer {
        use iced::advanced::renderer::Headless;
        let renderer = iced::futures::executor::block_on(<iced::Renderer as Headless>::new(
            crate::theme::FONT_MEDIUM,
            iced::Pixels(crate::theme::SIZE_BODY),
            Some("tiny-skia"),
        ))
        .expect("the tiny-skia renderer needs no display");
        for bytes in [
            crate::theme::FONT_BYTES_MEDIUM,
            crate::theme::FONT_BYTES_SEMIBOLD,
            crate::theme::FONT_BYTES_BOLD,
            crate::theme::FONT_BYTES_EXTRABOLD,
        ] {
            iced::advanced::graphics::text::font_system()
                .write()
                .expect("font system lock")
                .load_font(std::borrow::Cow::Borrowed(bytes));
        }
        renderer
    }

    /// Lay one block row out at `width` px wide.
    fn lay_out_row(r: &BlockRow, width: f32) -> iced::advanced::layout::Node {
        use iced::advanced::layout::Limits;
        use iced::advanced::widget::Tree;
        let renderer = headless();
        let mut element = block_row(r);
        let mut tree = Tree::new(&element);
        element.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &Limits::new(iced::Size::ZERO, iced::Size::new(width, 400.0)),
        )
    }

    /// Every leaf under `node` as (top, height), with `top` from the row's
    /// top edge.
    fn leaves(node: &iced::advanced::layout::Node, y: f32, out: &mut Vec<(f32, f32)>) {
        let b = node.bounds();
        let top = y + b.y;
        if node.children().is_empty() {
            out.push((top, b.height));
        }
        for c in node.children() {
            leaves(c, top, out);
        }
    }

    /// The middle of the one text leaf (shorter than a chip) under `node`.
    fn text_middle(node: &iced::advanced::layout::Node, y: f32) -> f32 {
        let mut out = Vec::new();
        leaves(node, y, &mut out);
        let words: Vec<_> = out.into_iter().filter(|(_, h)| *h < CHIP_H - 1.0).collect();
        assert_eq!(words.len(), 1, "one text leaf: {words:?}");
        words[0].0 + words[0].1 / 2.0
    }

    /// A row that is only a quiet line keeps its words level with its
    /// label, like a row of chips.
    #[test]
    fn a_quiet_only_row_is_level_with_its_label() {
        let r = BlockRow {
            label: "Bans",
            parts: vec![Part::Quiet {
                text: "No ban screen seen",
                tip: None,
            }],
        };
        let node = lay_out_row(&r, 1200.0);
        let [label, body] = node.children() else {
            panic!("a label and a body");
        };
        assert!(
            (body.bounds().height - CHIP_H).abs() < 0.5,
            "the body is a chip high: {:?}",
            body.bounds()
        );
        let at = node.bounds().y;
        let (label_mid, quiet_mid) = (text_middle(label, at), text_middle(body, at));
        assert!(
            (label_mid - quiet_mid).abs() < 1.0,
            "label middle {label_mid}, quiet middle {quiet_mid}"
        );
    }

    /// The mixed Bans row wraps onto two lines in the panel at 1024 (934px
    /// wide), and no group is squeezed to fit leftover width on a line.
    /// Natural widths come from a very wide one-line layout; iced would
    /// still keep every group's right edge inside the panel if wrap were
    /// off, so an edge check cannot catch a missing wrap.
    #[test]
    fn the_mixed_bans_row_wraps_inside_the_panel_at_1024() {
        let games = games_from_snapshot(&crate::fixtures_games::snapshot());
        let g = games.iter().find(|g| g.session_id == MIXED).expect("mixed");
        let BansPicks::Block(b) = crate::games_bans::bans_picks(g) else {
            panic!("mixed: no block");
        };
        let r = b.rows.iter().find(|r| r.label == "Bans").expect("Bans row");
        let width = 934.0;
        let node = lay_out_row(r, width);
        let wide = lay_out_row(r, 10_000.0);
        let [_, body] = node.children() else {
            panic!("a label and a body");
        };
        let [_, wide_body] = wide.children() else {
            panic!("a label and a body");
        };
        let natural: Vec<f32> = wide_body
            .children()
            .iter()
            .map(|unit| unit.bounds().width)
            .collect();
        assert_eq!(body.children().len(), natural.len());
        let mut tops: Vec<i32> = Vec::new();
        for (i, unit) in body.children().iter().enumerate() {
            let u = unit.bounds();
            assert!(
                (u.width - natural[i]).abs() < 0.5,
                "group {i} was squeezed: width {} vs natural {}",
                u.width,
                natural[i]
            );
            let top = u.y.round() as i32;
            if !tops.contains(&top) {
                tops.push(top);
            }
        }
        assert_eq!(tops.len(), 2, "two lines: {tops:?}");
        let mut wide_tops: Vec<i32> = Vec::new();
        for unit in wide_body.children() {
            let top = unit.bounds().y.round() as i32;
            if !wide_tops.contains(&top) {
                wide_tops.push(top);
            }
        }
        assert_eq!(wide_tops.len(), 1, "one line when wide: {wide_tops:?}");
    }
}
