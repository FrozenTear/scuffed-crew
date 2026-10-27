//! The six flat stat tiles (spec p1 and p2): E, D, A, DMG, HEAL and MIT,
//! each with its label and value, and an amber outline with "⚠ UNSURE" on
//! a value the reader was not sure of and no hand edit has fixed since.
//!
//! One shared component for the Games expanded row and the Overview's
//! newest game. Callers use [`row`] only, so Tracker's shared tiles from
//! #556 can replace this module whole.
//!
//! TODO(#557): a local shim of #557's `stat_tiles.rs` (head 47d8d0fa), so
//! the Games panel builds on the same API before #557 lands. #557 owns the
//! module. When it lands, take its copy and keep only the two changes
//! marked SHIM below: [`unsure_now`] reads the reader's flat and row names
//! ("dmg", "r3.dmg") through `stat_tracker::suspect`, and [`tile_with`] /
//! [`TileExtras`] add the lines the Games v2 panel puts in a tile. [`row`] and the tile look are #557's, unchanged.

use iced::widget::{Column, Row, column, container, image, text};
use iced::{Alignment, Background, Border, Color, Element, Fill, Padding, Theme};

use crate::app::Message;
use crate::icons::{self, Icon};
use crate::model::Game;
use crate::theme::{self, BORDER, FONT_BOLD, SIZE_LABEL, TEXT, TEXT_3};

/// The six stat tiles: label, the field the reader marks, and the value.
const TILES: [(&str, &str); 6] = [
    ("E", "elims"),
    ("D", "deaths"),
    ("A", "assists"),
    ("DMG", "damage"),
    ("HEAL", "healing"),
    ("MIT", "mitigation"),
];

/// The six tiles for `g`, wrapping on a narrow page.
pub fn row(g: &Game) -> Element<'static, Message> {
    let mut tiles = Row::new().spacing(8);
    for (label, field) in TILES {
        tiles = tiles.push(tile(label, stat(g, field), unsure_now(g, field)));
    }
    tiles.wrap().into()
}

/// A field the reader was unsure of that no hand edit has fixed since.
///
/// SHIM: the reader stores flat names in `suspect_fields` ("dmg", "h", or
/// a row name like "r3.dmg" on older rows), while `field` here is the edit
/// name ("damage"). [`crate::games_rows::unsure_now`] maps both through
/// `stat_tracker::suspect`. #557's copy checks only `field` and misses a
/// real "dmg".
pub fn unsure_now(g: &Game, field: &str) -> bool {
    crate::games_rows::unsure_now(g, field)
}

fn stat(g: &Game, field: &str) -> u32 {
    match field {
        "elims" => g.elims,
        "deaths" => g.deaths,
        "assists" => g.assists,
        "damage" => g.damage,
        "healing" => g.healing,
        _ => g.mitigation,
    }
}

/// "8.9k" from 1000 up.
pub fn short_num(n: u32) -> String {
    if n >= 1000 {
        format!("{:.1}k", f64::from(n) / 1000.0)
    } else {
        n.to_string()
    }
}

/// SHIM: what the Games v2 panel adds to a tile. The default adds nothing,
/// so `tile_with(.., TileExtras::default())` is the plain tile.
#[derive(Default)]
pub struct TileExtras {
    /// A badge in the label line, e.g. "EDITED" (accent outline, text).
    pub badge: Option<&'static str>,
    /// Lines under the value, e.g. "Read 22, you set 24" and "+6 vs avg".
    pub lines: Vec<(String, Color)>,
    /// Fill the width the parent gives (equal tiles in a grid).
    pub fill: bool,
}

/// A stat tile; an unsure one has an amber outline and "⚠ UNSURE".
fn tile(label: &'static str, value: u32, unsure: bool) -> Element<'static, Message> {
    tile_with(label, short_num(value), unsure, TileExtras::default())
}

/// SHIM: [`tile`] with a badge, lines under the value, and an equal width.
pub fn tile_with(
    label: &'static str,
    value: String,
    unsure: bool,
    extras: TileExtras,
) -> Element<'static, Message> {
    let mut head = Row::new().spacing(4).align_y(Alignment::Center).push(
        text(label)
            .size(SIZE_LABEL)
            .font(FONT_BOLD)
            .color(if unsure { theme::WARN } else { TEXT_3 }),
    );
    if unsure {
        head = head
            .push(image(icons::handle(Icon::Warn)).width(10).height(10))
            .push(
                text("UNSURE")
                    .size(SIZE_LABEL)
                    .font(FONT_BOLD)
                    .color(theme::WARN),
            );
    }
    if let Some(badge) = extras.badge {
        head = head.push(
            container(text(badge).size(11).font(FONT_BOLD).color(TEXT))
                .padding(Padding::from([0, 4]))
                .style(|_: &Theme| container::Style {
                    border: Border {
                        color: theme::ACCENT,
                        width: 1.0,
                        radius: 3.0.into(),
                    },
                    ..container::Style::default()
                }),
        );
    }
    let mut body: Column<'static, Message> =
        column![head, text(value).size(22).font(FONT_BOLD).color(TEXT)].spacing(4);
    for (line, color) in extras.lines {
        body = body.push(
            text(line)
                .size(12)
                .color(color)
                .wrapping(iced::widget::text::Wrapping::None),
        );
    }
    let mut c = container(body)
        .padding(Padding::from([10, 14]))
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(theme::BG)),
            border: Border {
                color: if unsure { theme::WARN } else { BORDER },
                width: 1.0,
                radius: 10.0.into(),
            },
            ..container::Style::default()
        });
    if extras.fill {
        c = c.width(Fill);
    }
    c.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_shorten_from_a_thousand() {
        assert_eq!(short_num(8_912), "8.9k");
        assert_eq!(short_num(28), "28");
    }

    #[test]
    fn a_hand_edit_settles_an_unsure_tile() {
        let mut g = crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
            crate::cli::FixtureKind::Sample,
        ))
        .remove(0);
        // SHIM: the reader stores "dmg", never the edit name "damage".
        g.suspect_fields = vec!["dmg".into()];
        g.edited_fields.clear();
        assert!(unsure_now(&g, "damage"));
        g.edited_fields = vec!["damage".into()];
        assert!(!unsure_now(&g, "damage"));
        let _ = row(&g);
    }

    /// SHIM: the reader writes flat names, so a real "dmg" reads unsure.
    #[test]
    fn the_readers_flat_names_read_unsure() {
        let mut g = crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
            crate::cli::FixtureKind::Sample,
        ))
        .remove(0);
        g.suspect_fields = vec!["dmg".into(), "h".into()];
        g.edited_fields.clear();
        assert!(unsure_now(&g, "damage"));
        assert!(unsure_now(&g, "healing"));
        assert!(!unsure_now(&g, "elims"));
        g.edited_fields = vec!["damage".into()];
        assert!(!unsure_now(&g, "damage"));
    }
}
